import assert from "node:assert/strict";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { spawnSync } from "node:child_process";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const workflows = new URL(".github/workflows/", root);
const files = readdirSync(workflows).filter(name => name.endsWith(".yml"));
const read = name => readFileSync(new URL(name, workflows), "utf8");

test("validation uses hosted runners, read-only permissions and pinned actions", () => {
  for (const name of files) {
    const source = read(name);
    assert.doesNotMatch(source, /self-hosted|pull_request_target|secrets:|:\s*write\b/, name);
    assert.match(source, /permissions:\n  contents: read/, name);
    for (const [, action] of source.matchAll(/uses: ([^\n]+)/g)) {
      if (action.startsWith("./")) {
        assert.ok(existsSync(new URL(action, root)), action);
      } else {
        assert.match(action, /^[\w-]+\/[\w-]+@[0-9a-f]{40} # v[\d.]+$/, action);
      }
    }
  }
  assert.match(read("ci.yml"), /push:\n    branches: \[main\]/);
  assert.match(read("ci.yml"), /pull_request:\n    branches: \[main\]/);
  assert.doesNotMatch(read("ci.yml"), /head.repo.full_name/);
});

test("the actual aggregate gate fails on failed or cancelled jobs, permits docs-only skips", () => {
  const source = read("validation.yml");
  assert.match(source, /name: Validation complete\n    needs:.*\n    if: always\(\)/);
  const code = source.split("<<'JS'\n")[1].split("\n          JS")[0];
  for (const [result, expected] of [["success", 0], ["skipped", 0], ["failure", 1], ["cancelled", 1]]) {
    const run = spawnSync(process.execPath, ["--input-type=module", "-e", code], {
      env: { ...process.env, RESULTS: JSON.stringify({ rust: { result }, frontend: { result: "success" } }) },
    });
    assert.equal(run.status, expected, result);
  }
});
