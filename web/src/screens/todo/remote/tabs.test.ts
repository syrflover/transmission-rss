import assert from "node:assert/strict";
import { test } from "node:test";

import type { Tab } from "./protocol.ts";
import { showsTabRow, tabLabel, UNNAMED_TAB, withShown } from "./tabs.ts";

const tab = (id: string, extra: Partial<Tab> = {}): Tab => ({ id, title: "", host: null, shown: false, closable: true, ...extra });

test("a tab is named by its title, else its host, else a plain name", () => {
  assert.equal(tabLabel(tab("a", { title: "자막 올림", host: "blog.example.org" })), "자막 올림");
  assert.equal(tabLabel(tab("a", { host: "blog.example.org" })), "blog.example.org");
  assert.equal(tabLabel(tab("a")), UNNAMED_TAB);
});

test("the row shows from two tabs on", () => {
  assert.equal(showsTabRow([]), false);
  assert.equal(showsTabRow([tab("a", { shown: true })]), false);
  assert.equal(showsTabRow([tab("a", { shown: true }), tab("b")]), true);
});

test("a switch that is on its way marks its page as shown until the server's tabs come", () => {
  const tabs = [tab("a", { shown: true, closable: false }), tab("b")];
  const marked = withShown(tabs, "b");
  assert.deepEqual(
    marked.map((t) => [t.id, t.shown]),
    [
      ["a", false],
      ["b", true],
    ],
  );
  // Nothing wanted, or a page that is not a tab: the tabs as they are.
  assert.equal(withShown(tabs, null), tabs);
  assert.equal(withShown(tabs, "gone"), tabs);
  assert.equal(withShown(tabs, "a"), tabs);
});
