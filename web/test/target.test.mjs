import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { TargetError, normalizeTarget } from "../src/lib/target.ts";

describe("target normalization", () => {
  it("adds https to bare hosts", () => {
    assert.equal(normalizeTarget("files.example.org/report.pdf"), "https://files.example.org/report.pdf");
  });

  it("supports protocol-relative URLs", () => {
    assert.equal(normalizeTarget("//files.example.org/report.pdf"), "https://files.example.org/report.pdf");
  });

  it("keeps a host:port instead of treating it as a scheme", () => {
    assert.equal(
      normalizeTarget("files.example.org:8443/report.pdf"),
      "https://files.example.org:8443/report.pdf",
    );
  });

  it("keeps explicit http URLs", () => {
    assert.equal(normalizeTarget("http://files.example.org/report.pdf"), "http://files.example.org/report.pdf");
  });

  it("normalizes an explicit scheme case-insensitively", () => {
    assert.equal(normalizeTarget("HTTPS://FILES.example.org/report.pdf"), "https://files.example.org/report.pdf");
  });

  it("rejects empty, control and non-http values", () => {
    for (const value of ["", "   ", "ftp://files.example.org/x", "https://files.example.org/a\u0000b"]) {
      assert.throws(() => normalizeTarget(value), TargetError, value);
    }
  });

  it("rejects credentials and fragments", () => {
    for (const value of [
      "https://user:pass@files.example.org/x",
      "https://files.example.org/x#fragment",
    ]) {
      assert.throws(() => normalizeTarget(value), TargetError, value);
    }
  });

  it("rejects URLs over the ALPS default limit", () => {
    const target = "https://files.example.org/" + "a".repeat(9000);
    assert.throws(() => normalizeTarget(target), TargetError);
  });

  it("exposes stable codes for localization", () => {
    const cases = [
      ["", "empty"],
      ["https://files.example.org/a\u0000b", "control"],
      ["ftp://files.example.org/x", "unsupported_scheme"],
      ["https://user:pass@files.example.org/x", "credentials"],
      ["https://files.example.org/x#fragment", "fragment"],
    ];

    for (const [value, code] of cases) {
      assert.throws(
        () => normalizeTarget(value),
        (error) => error instanceof TargetError && error.code === code,
      );
    }
  });
});
