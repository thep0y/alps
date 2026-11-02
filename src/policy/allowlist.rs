//! Allowlist rule compilation and host matching.

use crate::config::AllowlistRule;
use crate::policy::Scheme;

#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub host: String,
    pub include_subdomains: bool,
    pub schemes: Vec<Scheme>,
    pub ports: Vec<u16>,
    pub path_prefixes: Vec<String>,
    suffix: String,
}

impl CompiledRule {
    fn matches(&self, host: &str, scheme: Scheme, port: u16, path: &str) -> bool {
        if !self.ports.contains(&port) || !self.schemes.contains(&scheme) {
            return false;
        }
        if !self.path_prefixes.is_empty()
            && !self
                .path_prefixes
                .iter()
                .any(|prefix| path.starts_with(prefix))
        {
            return false;
        }
        if self.include_subdomains {
            host == self.host || host.ends_with(&self.suffix)
        } else {
            host == self.host
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Allowlist {
    rules: Vec<CompiledRule>,
}

impl Allowlist {
    /// Compile rules that have already been normalized by config validation.
    pub fn compile(rules: &[AllowlistRule]) -> Self {
        let rules = rules
            .iter()
            .filter_map(|rule| {
                let host = crate::config::normalize_host(&rule.host)?;
                let schemes = rule
                    .schemes
                    .iter()
                    .filter_map(|scheme| Scheme::parse(scheme))
                    .collect::<Vec<_>>();
                if schemes.is_empty() || rule.ports.is_empty() {
                    return None;
                }
                Some(CompiledRule {
                    suffix: format!(".{host}"),
                    host,
                    include_subdomains: rule.include_subdomains,
                    schemes,
                    ports: rule.ports.clone(),
                    path_prefixes: rule.path_prefixes.clone(),
                })
            })
            .collect();
        Self { rules }
    }

    pub fn permits(&self, host: &str, scheme: Scheme, port: u16, path: &str) -> bool {
        self.rules
            .iter()
            .any(|rule| rule.matches(host, scheme, port, path))
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AllowlistRule;

    fn rule(host: &str, subdomains: bool) -> AllowlistRule {
        AllowlistRule {
            host: host.to_string(),
            include_subdomains: subdomains,
            schemes: vec!["https".into()],
            ports: vec![443],
            path_prefixes: Vec::new(),
        }
    }

    fn list() -> Allowlist {
        Allowlist::compile(&[
            rule("files.example.org", false),
            rule("cdn.example.net", true),
        ])
    }

    #[test]
    fn exact_match_is_required_without_subdomain_flag() {
        let list = list();
        assert!(list.permits("files.example.org", Scheme::Https, 443, "/"));
        assert!(!list.permits("sub.files.example.org", Scheme::Https, 443, "/"));
        assert!(!list.permits("files.example.org.evil.example", Scheme::Https, 443, "/"));
        assert!(!list.permits("files.example.or", Scheme::Https, 443, "/"));
    }

    #[test]
    fn subdomain_match_respects_label_boundaries() {
        let list = list();
        assert!(list.permits("cdn.example.net", Scheme::Https, 443, "/"));
        assert!(list.permits("a.b.cdn.example.net", Scheme::Https, 443, "/"));
        assert!(!list.permits("notcdn.example.net", Scheme::Https, 443, "/"));
        assert!(!list.permits("cdn.example.net.evil", Scheme::Https, 443, "/"));
    }

    #[test]
    fn scheme_and_port_must_match_same_rule() {
        let list = list();
        assert!(!list.permits("files.example.org", Scheme::Http, 443, "/"));
        assert!(!list.permits("files.example.org", Scheme::Https, 8443, "/"));
    }

    #[test]
    fn empty_allowlist_permits_nothing() {
        let list = Allowlist::compile(&[]);
        assert!(list.is_empty());
        assert!(!list.permits("files.example.org", Scheme::Https, 443, "/"));
    }

    #[test]
    fn path_prefixes_are_literal_and_optional() {
        let mut release = rule("files.example.org", false);
        release.path_prefixes = vec!["/releases/".into(), "/pub/".into()];
        let list = Allowlist::compile(&[release]);

        assert!(list.permits("files.example.org", Scheme::Https, 443, "/releases/app.tar"));
        assert!(list.permits("files.example.org", Scheme::Https, 443, "/pub/x"));
        assert!(list.permits("files.example.org", Scheme::Https, 443, "/releases/"));
        assert!(!list.permits("files.example.org", Scheme::Https, 443, "/releases"));
        assert!(!list.permits("files.example.org", Scheme::Https, 443, "/releases-evil"));
        assert!(!list.permits("files.example.org", Scheme::Https, 443, "/other"));
    }
}
