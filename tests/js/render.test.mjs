import { test } from "node:test";
import assert from "node:assert/strict";
import { formatAmount, formatProgress, fillPercent, formatRemaining } from "../../src/donation_goal/static/render.js";

test("formatAmount groups thousands with spaces and drops fraction", () => {
  assert.equal(formatAmount(5000), "5 000");
  assert.equal(formatAmount(33825.61), "33 825");
  assert.equal(formatAmount(1234567), "1 234 567");
  assert.equal(formatAmount(0), "0");
  assert.equal(formatAmount(-1500), "-1 500");
});

test("formatProgress covers all formats", () => {
  assert.equal(formatProgress(5000, 10000, "percent"), "50%");
  assert.equal(formatProgress(5000, 10000, "amount"), "5 000 RUB");
  assert.equal(formatProgress(5000, 10000, "amount_percent"), "5 000 RUB (50%)");
  assert.equal(formatProgress(5000, 10000, "amount_target"), "5 000 / 10 000 RUB");
  assert.equal(formatProgress(5000, 10000, "amount_target_percent"), "5 000 / 10 000 RUB (50%)");
});

test("percent rounds down", () => {
  assert.equal(formatProgress(9999, 10000, "percent"), "99%");
});

test("test_percent_clamped", () => {
  assert.equal(fillPercent(33825.61, 5000), 100);
  assert.equal(fillPercent(-5, 5000), 0);
  assert.equal(fillPercent(2500, 5000), 50);
  assert.equal(formatProgress(33825.61, 5000, "percent"), "676%");
});

test("formatRemaining", () => {
  const now = new Date("2026-11-21T00:00:00Z");
  assert.equal(formatRemaining(new Date("2026-11-23T04:12:33Z"), now), "Осталось 2 д 04:12:33");
  assert.equal(formatRemaining(new Date("2026-11-21T04:12:33Z"), now), "Осталось 04:12:33");
  assert.equal(formatRemaining(new Date("2026-11-20T00:00:00Z"), now), "Сбор завершён");
  assert.equal(formatRemaining(null, now), "");
});
