import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { detectLocale, messagesFor } from "../src/lib/i18n.ts";

describe("locale detection", () => {
  it("uses Chinese for any zh tag", () => {
    assert.equal(detectLocale(["zh-CN"]), "zh");
    assert.equal(detectLocale(["zh-Hant-TW"]), "zh");
    assert.equal(detectLocale(["zh"]), "zh");
  });

  it("uses English for any en tag", () => {
    assert.equal(detectLocale(["en-US"]), "en");
    assert.equal(detectLocale(["en-GB"]), "en");
    assert.equal(detectLocale(["en"]), "en");
  });

  it("picks the first supported browser language", () => {
    assert.equal(detectLocale(["fr-FR", "zh-CN"]), "zh");
    assert.equal(detectLocale(["fr-FR", "en-US"]), "en");
    assert.equal(detectLocale(["en-US", "zh-CN"]), "en");
  });

  it("falls back to English", () => {
    assert.equal(detectLocale(undefined), "en");
    assert.equal(detectLocale([]), "en");
    assert.equal(detectLocale(["fr-FR", "de-DE"]), "en");
  });
});

describe("translations", () => {
  it("uses ALPS as the title and brand in both locales", () => {
    for (const locale of ["en", "zh"]) {
      const messages = messagesFor(locale);
      assert.equal(messages.pageTitle, "ALPS");
      assert.equal(messages.brand, "ALPS");
    }
  });

  it("keeps both dictionaries aligned", () => {
    const en = messagesFor("en");
    const zh = messagesFor("zh");
    assert.deepEqual(Object.keys(en).sort(), Object.keys(zh).sort());
    assert.deepEqual(Object.keys(en.targetErrors).sort(), Object.keys(zh.targetErrors).sort());
    assert.deepEqual(Object.keys(en.proxyErrors).sort(), Object.keys(zh.proxyErrors).sort());
    assert.deepEqual(Object.keys(en.statusErrors).sort(), Object.keys(zh.statusErrors).sort());
    assert.deepEqual(Object.keys(en.features).sort(), Object.keys(zh.features).sort());
  });
});
