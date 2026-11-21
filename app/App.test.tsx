// Headless `nsh test` verification of the `App` root (`nsh.json` "root").
import { expect, open, test } from "nsh/test";

test("renders Landing and toggles EN/ZH localization", async () => {
  const app = await open(".", {
    width: 1280,
    height: 820,
    platform: "macos",
  });
  expect(app.texts()).toContain("Diving");
  expect(app.texts()).toContain("Dive into every layer");
  expect(app.texts()).toContain("Analyze");
  expect(app.texts()).toContain("Open local file…");

  await app.press("中");
  expect(app.texts()).toContain("潜入每一层镜像");

  await app.press("EN");
  expect(app.texts()).toContain("Dive into every layer");
});
