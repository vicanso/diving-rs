// Light/dark desktop screenshot verification (`nsh test gallery.test.tsx --target macos`).
import { expect, open, test } from "nsh/test";

const FIXTURE_REF = "file://.nsh/out/fixture.tar";
const FIXTURE_V2_REF = "file://.nsh/out/fixture-v2.tar";

async function captureIfGraphical(
  app: Awaited<ReturnType<typeof open>>,
  path: string,
) {
  try {
    await app.screenshot(path);
  } catch (err) {
    // `nsh test` runs headless unless `--target` is passed; screenshots render only with `--target`.
    if (!String(err).includes("--target")) {
      throw err;
    }
  }
}

test("landing renders light theme screenshot on macos", async () => {
  const app = await open(".", {
    width: 1280,
    height: 820,
    colorScheme: "light",
    platform: "macos",
  });
  expect(app.texts()).toContain("Diving");
  expect(app.texts()).toContain("Dive into every layer");
  await app.waitFor("Analyze");
  await captureIfGraphical(app, ".nsh/out/landing-light.png");
});

test("landing renders dark theme screenshot on macos", async () => {
  const app = await open(".", {
    width: 1280,
    height: 820,
    colorScheme: "dark",
    platform: "macos",
  });
  expect(app.texts()).toContain("Diving");
  expect(app.texts()).toContain("Dive into every layer");
  await app.waitFor("Analyze");
  await captureIfGraphical(app, ".nsh/out/landing-dark.png");
});

test("inspector renders light theme screenshot on macos", async () => {
  const app = await open(".", {
    width: 1280,
    height: 820,
    colorScheme: "light",
    platform: "macos",
  });
  await app.type("Image reference input", FIXTURE_REF);
  await app.press("Analyze");
  await app.waitFor("Layers");
  await app.press("Expand all");
  await app.waitFor("app.conf");
  await captureIfGraphical(app, ".nsh/out/inspector-light.png");
});

test("inspector renders dark theme screenshot on macos", async () => {
  const app = await open(".", {
    width: 1280,
    height: 820,
    colorScheme: "dark",
    platform: "macos",
  });
  await app.type("Image reference input", FIXTURE_REF);
  await app.press("Analyze");
  await app.waitFor("Layers");
  await app.press("Expand all");
  await app.waitFor("app.conf");
  await captureIfGraphical(app, ".nsh/out/inspector-dark.png");
});

test("treemap renders light theme screenshot on macos", async () => {
  const app = await open(".", {
    width: 1280,
    height: 820,
    colorScheme: "light",
    platform: "macos",
  });
  await app.type("Image reference input", FIXTURE_REF);
  await app.press("Analyze");
  await app.waitFor("Layers");
  await app.press("Treemap");
  await app.waitFor("Root");
  await captureIfGraphical(app, ".nsh/out/treemap-light.png");
});

test("treemap renders dark theme screenshot on macos", async () => {
  const app = await open(".", {
    width: 1280,
    height: 820,
    colorScheme: "dark",
    platform: "macos",
  });
  await app.type("Image reference input", FIXTURE_REF);
  await app.press("Analyze");
  await app.waitFor("Layers");
  await app.press("Treemap");
  await app.waitFor("Root");
  await captureIfGraphical(app, ".nsh/out/treemap-dark.png");
});

test("diff renders light theme screenshot on macos", async () => {
  const app = await open(".", {
    width: 1280,
    height: 820,
    colorScheme: "light",
    platform: "macos",
  });
  await app.keyboard.press("mod+d");
  await app.waitFor("Base image");
  await app.type("Base image input", FIXTURE_REF);
  await app.type("Target image input", FIXTURE_V2_REF);
  await app.press("Compare");
  await app.waitFor("text:/^Where the Size Changed/");
  await captureIfGraphical(app, ".nsh/out/diff-light.png");
});

test("diff renders dark theme screenshot on macos", async () => {
  const app = await open(".", {
    width: 1280,
    height: 820,
    colorScheme: "dark",
    platform: "macos",
  });
  await app.keyboard.press("mod+d");
  await app.waitFor("Base image");
  await app.type("Base image input", FIXTURE_REF);
  await app.type("Target image input", FIXTURE_V2_REF);
  await app.press("Compare");
  await app.waitFor("text:/^Where the Size Changed/");
  await captureIfGraphical(app, ".nsh/out/diff-dark.png");
});

test("compact inspector renders cleanly at 960x640 on macos", async () => {
  const app = await open(".", {
    width: 960,
    height: 640,
    colorScheme: "light",
    platform: "macos",
  });
  await app.type("Image reference input", FIXTURE_REF);
  await app.press("Analyze");
  await app.waitFor("Layers");
  await captureIfGraphical(app, ".nsh/out/inspector-compact-960.png");
});

test("inspector renders cleanly with reducedMotion on macos", async () => {
  const app = await open(".", {
    width: 1280,
    height: 820,
    colorScheme: "dark",
    reducedMotion: true,
    platform: "macos",
  });
  await app.type("Image reference input", FIXTURE_REF);
  await app.press("Analyze");
  await app.waitFor("Layers");
  await app.press("Treemap");
  await app.waitFor("Root");
});
