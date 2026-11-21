//! Headless end-to-end integration tests for the Diving Desktop `App` M2 Inspector workflows.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use nsh_app_diving::App;
use nsh_app_diving::ui::app_tokens;
use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Root, Size};
use nsh_testing::Headless;
use tar::{Builder, Header};
use tempfile::Builder as TempBuilder;

fn tar_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = Builder::new(Vec::new());
    for (path, data) in files {
        let mut header = Header::new_gnu();
        header.set_path(path).expect("set tar header path");
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, *data).expect("append tar entry");
    }
    builder.into_inner().expect("finish tar archive")
}

fn docker_save_tar(layer1: &[u8], layer2: &[u8], layer2_diff_id: &str, repo_tag: &str) -> Vec<u8> {
    let config = format!(
        r#"{{
        "architecture": "amd64",
        "created": "2026-01-01T00:00:00Z",
        "history": [
            {{"created": "2026-01-01T00:00:00Z", "created_by": "/bin/sh -c #(nop) COPY layer1 /"}},
            {{"created": "2026-01-01T00:00:00Z", "created_by": "/bin/sh -c #(nop)  ENV APP_ENV=production", "empty_layer": true}},
            {{"created": "2026-01-01T00:00:00Z", "created_by": "/bin/sh -c #(nop) COPY app/ /app"}}
        ],
        "os": "linux",
        "rootfs": {{"type": "layers", "diff_ids": ["sha256:1111", "{layer2_diff_id}"]}},
        "config": {{
            "User": "",
            "Env": ["PATH=/usr/local/bin:/usr/bin", "APP_ENV=production"],
            "Entrypoint": ["/usr/bin/app"],
            "Cmd": []
        }}
    }}"#
    );

    let manifest = format!(
        r#"[{{
        "Config": "config.json",
        "RepoTags": ["{repo_tag}"],
        "Layers": ["layer1/layer.tar", "layer2/layer.tar"]
    }}]"#
    );

    tar_bytes(&[
        ("manifest.json", manifest.as_bytes()),
        ("config.json", config.as_bytes()),
        ("layer1/layer.tar", layer1),
        ("layer2/layer.tar", layer2),
    ])
}

fn fixture_layer1() -> Vec<u8> {
    let mut elf_payload = vec![0u8; 64];
    elf_payload[..8].copy_from_slice(&[0x7f, b'E', b'L', b'F', 0x00, 0x01, 0x02, 0x03]);

    tar_bytes(&[
        ("etc/app.conf", b"listen = 0.0.0.0:8080\n"),
        ("usr/bin/old", b"legacy-bin"),
        ("usr/bin/app", &elf_payload),
    ])
}

fn binary_fixture_payload() -> Vec<u8> {
    let mut binary_payload = Vec::with_capacity(128);
    for _ in 0..32 {
        binary_payload.extend_from_slice(&[0x00, 0xff, 0x01, 0x80]);
    }
    binary_payload
}

fn build_test_image_tar() -> Vec<u8> {
    let layer1 = fixture_layer1();
    let binary_payload = binary_fixture_payload();
    let layer2 = tar_bytes(&[
        ("usr/bin/.wh.old", b""),
        ("app/.env", b"SECRET_KEY=demo\n"),
        ("data/payload.bin", &binary_payload),
    ]);
    docker_save_tar(&layer1, &layer2, "sha256:2222", "diving-fixture:latest")
}

fn build_test_image_v2_tar() -> Vec<u8> {
    let layer1 = fixture_layer1();
    let binary_payload = binary_fixture_payload();
    let extra_payload = vec![3u8; 4096];
    let layer2 = tar_bytes(&[
        ("usr/bin/.wh.old", b""),
        (
            "etc/app.conf",
            b"listen = 0.0.0.0:8080\nworkers = 4\ntimeout_ms = 5000\n",
        ),
        ("data/payload.bin", &binary_payload),
        ("data/extra.bin", &extra_payload),
    ]);
    docker_save_tar(&layer1, &layer2, "sha256:3333", "diving-fixture:v2")
}

fn start_desktop_app() -> Headless {
    let config = EngineConfig {
        environment: Environment {
            os: Os::MacOs,
            idiom: Idiom::Desktop,
            ..Default::default()
        },
        viewport: Size::new(Points(1280.0), Points(820.0)),
        ..EngineConfig::new(Root::of::<App>().tokens(app_tokens()))
    };

    let mut app = Headless::start(config).expect("start headless App");
    app.set_viewport(Size::new(Points(1280.0), Points(820.0)))
        .expect("set desktop viewport");
    app
}

fn viewer_contains(app: &Headless, selector: &str, needle: &str) -> bool {
    if let Ok(Some(value)) = app.screen().input_text(selector)
        && value.contains(needle)
    {
        return true;
    }
    app.screen().texts().join(" ").contains(needle)
}

#[test]
fn landing_analyzes_local_tar_and_inspects_files_findings_and_dockerfile() {
    let tmp = TempBuilder::new()
        .suffix(".tar")
        .tempfile()
        .expect("create temporary fixture tar");
    fs::write(tmp.path(), build_test_image_tar()).expect("write fixture tar");

    let image_ref = format!("file://{}", tmp.path().display());
    let mut app = start_desktop_app();

    assert!(app.shows("Dive into every layer"));
    assert!(app.shows("Analyze"));
    assert!(app.shows("Open local file…"));

    app.type_text("Image reference input", &image_ref)
        .expect("type image reference");
    app.press("Analyze").expect("press Analyze");
    app.wait_for_within("Layers", Duration::from_secs(10))
        .expect("transition to Inspector view");

    assert!(app.shows("Efficiency"));
    assert!(app.shows("Image Size"));
    assert!(app.shows("Wasted"));
    assert!(app.shows("File Tree (3)"));
    assert!(app.shows("+3 / ~0 / −0"));

    app.press("Expand all").expect("expand all file tree dirs");
    app.wait_for_within("app.conf", Duration::from_secs(3))
        .expect("show expanded app.conf entry");

    app.press("app.conf")
        .expect("open preview sheet for app.conf");
    app.wait_for_within("Layer #1", Duration::from_secs(5))
        .expect("show file preview header badge");
    app.wait_for_within("File text content viewer", Duration::from_secs(5))
        .expect("mount text file viewer");
    assert!(
        viewer_contains(&app, "File text content viewer", "listen = 0.0.0.0:8080"),
        "expected app.conf contents inside preview viewer"
    );

    app.press_key("escape").expect("close preview on Escape");
    app.wait_for_gone("Layer #1", Duration::from_secs(3))
        .expect("dismiss preview sheet");

    app.press("text:/^Findings/")
        .expect("switch to Findings tab");
    app.wait_for_within(
        "text:/^Optimization Recommendations/",
        Duration::from_secs(5),
    )
    .expect("render Optimization Recommendations panel");
    assert_eq!(
        app.screen().matches("text:/^Wasted Space/"),
        Ok(true),
        "expected Wasted Space panel"
    );
    assert_eq!(
        app.screen().matches("text:/^Sensitive Files/"),
        Ok(true),
        "expected Sensitive Files panel"
    );
    assert!(app.shows("app/.env"), "expected app/.env sensitive finding");

    app.press("Dockerfile").expect("switch to Dockerfile tab");
    app.wait_for_within("Reconstructed Dockerfile viewer", Duration::from_secs(5))
        .expect("mount reconstructed Dockerfile viewer");
    assert!(
        viewer_contains(
            &app,
            "Reconstructed Dockerfile viewer",
            "ENV APP_ENV=production"
        ) || viewer_contains(&app, "Reconstructed Dockerfile viewer", "ENTRYPOINT"),
        "expected Dockerfile directives in viewer"
    );
}

#[test]
fn language_toggle_switches_en_and_zh() {
    let mut app = start_desktop_app();
    assert!(app.shows("Dive into every layer"));
    assert!(app.shows("Analyze"));

    app.press("中").expect("switch language to ZH");
    assert!(app.shows("潜入每一层镜像"));
    assert!(app.shows("开始分析"));
    assert!(app.shows("打开本地文件…"));

    app.press("EN").expect("switch language back to EN");
    assert!(app.shows("Dive into every layer"));
    assert!(app.shows("Analyze"));
}

#[test]
fn invalid_image_shows_inline_error_and_preserves_input() {
    let missing_ref = "file:///nonexistent/diving-missing-fixture.tar";
    let mut app = start_desktop_app();

    app.type_text("Image reference input", missing_ref)
        .expect("type missing fixture reference");
    app.press("Analyze").expect("submit invalid image");
    app.wait_for_within("Retry", Duration::from_secs(5))
        .expect("display inline retryable error banner");

    assert!(app.shows("Dive into every layer"));
    assert_eq!(
        app.screen().input_text("Image reference input"),
        Ok(Some(missing_ref))
    );
}

#[test]
fn treemap_drills_down_and_switches_to_table_view() {
    let tmp = TempBuilder::new()
        .suffix(".tar")
        .tempfile()
        .expect("create temporary fixture tar");
    fs::write(tmp.path(), build_test_image_tar()).expect("write fixture tar");

    let image_ref = format!("file://{}", tmp.path().display());
    let mut app = start_desktop_app();
    app.type_text("Image reference input", &image_ref)
        .expect("type image ref");
    app.press("Analyze").expect("press Analyze");
    app.wait_for_within("Layers", Duration::from_secs(10))
        .expect("enter Inspector");

    app.press("Treemap").expect("switch to Treemap tab");
    app.wait_for_within("Root", Duration::from_secs(5))
        .expect("render Treemap root toolbar");
    app.wait_for_within("usr", Duration::from_secs(3))
        .expect("show usr treemap block");

    app.press("usr").expect("drill down into usr directory");
    app.wait_for_within("bin", Duration::from_secs(3))
        .expect("render child bin inside usr breadcrumb");

    app.press("Root").expect("navigate back to Root");
    app.wait_for_within("data", Duration::from_secs(3))
        .expect("show data tile back at Root");

    app.press("Table").expect("switch Treemap to Table view");
    app.wait_for_within("data", Duration::from_secs(3))
        .expect("render data row in Treemap Table");
    assert!(app.shows("usr"));
}

#[test]
fn diff_compares_two_local_tars_and_swaps() {
    let tmp_v1 = TempBuilder::new()
        .suffix(".tar")
        .tempfile()
        .expect("create v1 fixture");
    let tmp_v2 = TempBuilder::new()
        .suffix(".tar")
        .tempfile()
        .expect("create v2 fixture");
    fs::write(tmp_v1.path(), build_test_image_tar()).expect("write v1 tar");
    fs::write(tmp_v2.path(), build_test_image_v2_tar()).expect("write v2 tar");

    let ref_v1 = format!("file://{}", tmp_v1.path().display());
    let ref_v2 = format!("file://{}", tmp_v2.path().display());

    let mut app = start_desktop_app();
    app.press_key("mod+d").expect("open Compare with dialog");
    app.wait_for_within("Base image", Duration::from_secs(3))
        .expect("render Compare dialog");

    app.type_text("Base image input", &ref_v1)
        .expect("type base image");
    app.type_text("Target image input", &ref_v2)
        .expect("type target image");
    app.press("Compare").expect("submit diff comparison");

    app.wait_for_within("text:/^Where the Size Changed/", Duration::from_secs(10))
        .expect("render Diff workspace");
    assert!(app.shows("Added (1)"), "expected Added (1) bucket");
    assert!(app.shows("Removed (1)"), "expected Removed (1) bucket");
    assert!(app.shows("Resized (1)"), "expected Resized (1) bucket");

    app.press("Swap").expect("swap Base and Target images");
    app.wait_for_within("Added (1)", Duration::from_secs(3))
        .expect("recompute swapped diff buckets");
    assert!(app.shows("Removed (1)"));
}

#[test]
fn command_palette_opens_with_mod_k_and_switches_language() {
    let mut app = start_desktop_app();
    assert!(app.shows("Dive into every layer"));

    app.press_key("mod+k").expect("open Command Palette");
    app.wait_for_within("Command Palette", Duration::from_secs(3))
        .expect("display Command Palette dialog");

    app.type_text("Command Palette", "chinese")
        .expect("filter for Chinese language action");
    app.wait_for_within("Language: 中文", Duration::from_secs(3))
        .expect("show filtered Chinese language command");
    app.press("Language: 中文")
        .expect("execute Language: 中文 command");

    app.wait_for_within("潜入每一层镜像", Duration::from_secs(3))
        .expect("localize UI to ZH");
    app.wait_for_gone("Command Palette", Duration::from_secs(3))
        .expect("close Command Palette after selection");
}

#[test]
fn local_picker_dialog_opens_and_closes_on_escape() {
    let mut app = start_desktop_app();
    app.press("Browse Docker / Podman…")
        .expect("open Local Images picker");
    app.wait_for_within("Local Images", Duration::from_secs(3))
        .expect("render Local Images dialog");
    assert!(app.shows("Docker"));
    assert!(app.shows("Podman"));

    app.wait_until(
        "local images loaded or recovery shown",
        Duration::from_secs(8),
        |screen| {
            screen.shows("Repository:Tag")
                || screen.shows("Unable to list local images")
                || screen.shows("No local images matched.")
        },
    )
    .expect("finish listing local images or display daemon recovery");

    app.press_key("escape")
        .expect("dismiss Local Images on Esc");
    app.wait_for_gone("Local Images", Duration::from_secs(3))
        .expect("close Local Images dialog");
}

#[test]
fn compact_viewport_and_reduced_motion_render_cleanly() {
    let tmp = TempBuilder::new()
        .suffix(".tar")
        .tempfile()
        .expect("create temporary fixture tar");
    fs::write(tmp.path(), build_test_image_tar()).expect("write fixture tar");

    let config = EngineConfig {
        environment: Environment {
            os: Os::MacOs,
            idiom: Idiom::Desktop,
            reduce_motion: true,
            ..Default::default()
        },
        viewport: Size::new(Points(960.0), Points(640.0)),
        ..EngineConfig::new(Root::of::<App>().tokens(app_tokens()))
    };

    let mut app = Headless::start(config).expect("start compact reduced-motion App");
    app.set_viewport(Size::new(Points(960.0), Points(640.0)))
        .expect("set compact 960x640 viewport");

    let image_ref = format!("file://{}", tmp.path().display());
    app.type_text("Image reference input", &image_ref)
        .expect("type image ref");
    app.press("Analyze").expect("submit image ref");
    app.wait_for_within("Layers", Duration::from_secs(10))
        .expect("render compact Inspector");
    assert!(app.shows("Efficiency"));

    app.press("Treemap").expect("switch to Treemap");
    app.wait_for_within("Root", Duration::from_secs(3))
        .expect("render compact Treemap");

    app.press("text:/^Findings/")
        .expect("switch to Findings tab");
    app.wait_for_within(
        "text:/^Optimization Recommendations/",
        Duration::from_secs(3),
    )
    .expect("render compact Findings");
}

#[test]
fn keyboard_arrow_keys_step_through_layers() {
    let tmp = TempBuilder::new()
        .suffix(".tar")
        .tempfile()
        .expect("create temporary fixture tar");
    fs::write(tmp.path(), build_test_image_tar()).expect("write fixture tar");

    let image_ref = format!("file://{}", tmp.path().display());
    let mut app = start_desktop_app();
    app.type_text("Image reference input", &image_ref)
        .expect("type image ref");
    app.press("Analyze").expect("press Analyze");
    app.wait_for_within("Layers", Duration::from_secs(10))
        .expect("enter Inspector");

    assert!(app.shows("#1"));
    assert!(app.shows("COPY layer1 /"));

    app.press_key("down").expect("step down to Layer #2");
    app.wait_for_within("#2", Duration::from_secs(3))
        .expect("select Layer #2");
    assert!(app.shows("ENV APP_ENV=production"));

    app.press_key("down").expect("step down to Layer #3");
    app.wait_for_within("#3", Duration::from_secs(3))
        .expect("select Layer #3");
    assert!(app.shows("COPY app/ /app"));

    app.press_key("up").expect("step back up to Layer #2");
    app.wait_for_within("#2", Duration::from_secs(3))
        .expect("re-select Layer #2");
}

fn write_fixture_if_env(var: &str, default_rel_path: &str, bytes: &[u8]) {
    let dest = env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(default_rel_path));
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).expect("create screenshot fixture parent directory");
    }
    fs::write(&dest, bytes).expect("write screenshot fixture tar");
}

#[test]
fn write_optional_screenshot_fixture() {
    write_fixture_if_env(
        "DIVING_WRITE_FIXTURE_TAR",
        ".nsh/out/fixture.tar",
        &build_test_image_tar(),
    );
    write_fixture_if_env(
        "DIVING_WRITE_FIXTURE_V2_TAR",
        ".nsh/out/fixture-v2.tar",
        &build_test_image_v2_tar(),
    );
}
