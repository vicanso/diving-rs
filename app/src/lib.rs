//! The app's Rust UI: its root component, which `nsh run`, `nsh package`
//! and `cargo run` mount.

pub mod domain;
pub mod features;
pub mod i18n;
pub mod ui;

use nsh_core::desktop::{TitleBarOptions, TitleBarStyle, WindowAttributes};
use nsh_core::element::desktop::{on_reopen, window as app_window};
use nsh_core::prelude::*;

use crate::domain::{AppRoute, AppState, DivingService, UpdatePhase};
use crate::features::{
    app_sidebar, bind_global_keys, command_palette_overlay, compare_dialog, content_header_bar,
    diff_workspace, file_preview_sheet, inspector_workspace, install_application_menu,
    landing_view, local_picker_dialog, provenance_dialog, registry_auth_dialog,
};
use crate::i18n::{t, tf};
use crate::ui::{
    BannerTone, ButtonVariant, IconName, UiSize, font_mono_family, register_fonts, theme,
    ui_banner, ui_button, ui_progress_bar,
};
#[cfg(debug_assertions)]
use crate::ui::{UiGallery, ui_gallery};

/// The app's UI set (`nsh.json` `"rust"`): what a TSX entry may import from
/// `diving/native` (`README.md`, "Adding a TSX entry").
#[native_module("diving/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
    #[cfg(debug_assertions)]
    registry.register::<UiGallery>();
}

/// The application's root component.
#[component]
pub fn App() -> impl IntoElement {
    register_fonts();

    let state = AppState::new(DivingService::new());
    state.startup_update_tasks();
    provide_context(state.clone());
    install_application_menu(state.clone());

    let main_open = signal(true);
    let mut attrs = WindowAttributes::default();
    if os().is_desktop() {
        let mut bar = TitleBarOptions::default();
        bar.style = TitleBarStyle::Hidden;
        bar.height = Some(px(48.0));

        attrs.title = Some("Diving".into());
        attrs.min_size = Some(Some(size(px(960.0), px(640.0))));
        attrs.title_bar = Some(bar);

        on_reopen(move || main_open.set(true));
    }

    let host_size = untracked(window);
    let initial_size = if host_size.width >= px(960.0) && host_size.height >= px(640.0) {
        host_size
    } else {
        size(px(1280.0), px(820.0))
    };

    let win_state = state;
    app_window()
        .id("main")
        .default_size(initial_size)
        .restore_bounds(true)
        .attributes(attrs)
        .open(main_open)
        .on_open_change(move |ev| main_open.set(ev.open))
        .content(move || {
            let state = win_state.clone();
            bind_global_keys(state.clone());
            let sidebar_open = state.app_sidebar_open;
            let sidebar_state = state.clone();
            let route_state = state.clone();
            let config_state = state.clone();
            let progress_state = state.clone();
            let drop_state = state.clone();
            let notice_state = state.clone();

            view()
                .relative()
                .flex_1()
                .flex_row()
                .bg(move || theme().background)
                .text_color(move || theme().foreground)
                .on_file_drop(move |ev: &FileDropEvent| {
                    if let Some(first) = ev.uris.first() {
                        drop_state.handle_dropped_uri(first);
                    }
                })
                .child(when(
                    move || sidebar_open.get(),
                    move || app_sidebar(sidebar_state.clone()),
                ))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .flex_col()
                        .bg(move || theme().background)
                        .child(content_header_bar(state.clone()))
                        .child(when(
                            move || config_state.config_warning.get().is_some(),
                            move || config_warning_banner(config_state.clone()),
                        ))
                        .child(when(
                            move || {
                                progress_state.is_loading.get()
                                    && progress_state.route.get() != AppRoute::Landing
                            },
                            move || global_progress_strip(progress_state.clone()),
                        ))
                        .child(view().flex_1().min_h(px(0.0)).flex_col().child(dynamic(
                            move || match route_state.route.get() {
                                AppRoute::Landing => {
                                    landing_view(route_state.clone()).into_element()
                                }
                                AppRoute::Inspector => {
                                    inspector_workspace(route_state.clone()).into_element()
                                }
                                AppRoute::Diff => {
                                    diff_workspace(route_state.clone()).into_element()
                                }
                                #[cfg(debug_assertions)]
                                AppRoute::Gallery => ui_gallery().into_element(),
                            },
                        ))),
                )
                .child(when(
                    move || notice_state.status_notice.get().is_some(),
                    move || floating_status_toast(notice_state.clone()),
                ))
                .child(file_preview_sheet(state.clone()))
                .child(compare_dialog(state.clone()))
                .child(local_picker_dialog(state.clone()))
                .child(provenance_dialog(state.clone()))
                .child(registry_auth_dialog(state.clone()))
                .child(command_palette_overlay(state))
        })
}

/// Renders the localized status notice banner with a close button (reused by floating toast and modals).
pub fn status_notice_banner(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let notice = state.status_notice;
    let dismiss_state = state;

    ui_banner(
        BannerTone::Info,
        move || SharedString::from(notice.get().unwrap_or_default()),
        Some(
            ui_button(
                move || SharedString::from(t(lang.get(), "common.close")),
                ButtonVariant::Ghost,
                UiSize::Xs,
                Some(IconName::Close),
                false,
                false,
                move || dismiss_state.dismiss_status_notice(),
            )
            .into_element(),
        ),
    )
}

fn floating_status_toast(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let is_update = state.status_notice_is_update;
    let phase = state.update_phase;
    let install_state = state.clone();
    let skip_state = state.clone();
    let restart_state = state.clone();
    let retry_state = state.clone();

    view()
        .absolute()
        .bottom_8()
        .right_4()
        .max_w(px(480.0))
        .shadow_md()
        .rounded_md()
        .bg(move || theme().popover)
        .flex_col()
        .child(status_notice_banner(state))
        .child(when(
            move || is_update.get(),
            move || match phase.get() {
                UpdatePhase::Available { .. } => {
                    let s1 = install_state.clone();
                    let s2 = skip_state.clone();
                    view()
                        .px_3()
                        .pb_2p5()
                        .flex_row()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "update.action_skip")),
                            ButtonVariant::Ghost,
                            UiSize::Xs,
                            None,
                            false,
                            false,
                            move || s2.skip_available_update(),
                        ))
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "update.action_install")),
                            ButtonVariant::Default,
                            UiSize::Xs,
                            Some(IconName::Download),
                            false,
                            false,
                            move || s1.start_update(),
                        ))
                        .into_element()
                }
                UpdatePhase::ReadyToRestart { .. } => {
                    let s = restart_state.clone();
                    view()
                        .px_3()
                        .pb_2p5()
                        .flex_row()
                        .justify_end()
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "update.restart_now")),
                            ButtonVariant::Default,
                            UiSize::Xs,
                            Some(IconName::Refresh),
                            false,
                            false,
                            move || s.relaunch_after_update(),
                        ))
                        .into_element()
                }
                UpdatePhase::Failed {
                    version: Some(_), ..
                } => {
                    let s = retry_state.clone();
                    view()
                        .px_3()
                        .pb_2p5()
                        .flex_row()
                        .justify_end()
                        .child(ui_button(
                            move || SharedString::from(t(lang.get(), "update.retry")),
                            ButtonVariant::Outline,
                            UiSize::Xs,
                            Some(IconName::Refresh),
                            false,
                            false,
                            move || s.start_update(),
                        ))
                        .into_element()
                }
                _ => view().into_element(),
            },
        ))
}

fn config_warning_banner(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let warning = state.config_warning;
    let dismiss_state = state;

    view().px_4().pt_2().child(ui_banner(
        BannerTone::Warning,
        move || {
            let err = warning.get().unwrap_or_default();
            SharedString::from(tf(lang.get(), "config.fallback_warning", &[&err]))
        },
        Some(
            ui_button(
                move || SharedString::from(t(lang.get(), "common.close")),
                ButtonVariant::Ghost,
                UiSize::Xs,
                Some(IconName::Close),
                false,
                false,
                move || dismiss_state.dismiss_config_warning(),
            )
            .into_element(),
        ),
    ))
}

fn global_progress_strip(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let pending = state.pending_image;
    let progress_text = state.progress_text;
    let cancel_state = state.clone();

    view()
        .w_full()
        .px_4()
        .py_2()
        .flex_row()
        .items_center()
        .gap_3()
        .bg(move || theme().muted)
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_col()
                .gap_1()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            text(move || {
                                let img = pending.get().unwrap_or_default();
                                if img.is_empty() {
                                    SharedString::from(t(lang.get(), "landing.analyzing"))
                                } else {
                                    SharedString::from(tf(
                                        lang.get(),
                                        "progress.analyzing_image",
                                        &[&img],
                                    ))
                                }
                            })
                            .text_xs()
                            .font_family(font_mono_family())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().foreground),
                        )
                        .child(
                            text(move || {
                                progress_text.get().unwrap_or_else(|| {
                                    t(lang.get(), "landing.analyzing").to_owned()
                                })
                            })
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(ui_progress_bar(
                    state.progress_ratio,
                    state.progress_indeterminate,
                    "Image analysis progress",
                )),
        )
        .child(ui_button(
            move || SharedString::from(t(lang.get(), "landing.cancel")),
            ButtonVariant::Outline,
            UiSize::Xs,
            Some(IconName::Close),
            false,
            false,
            move || cancel_state.cancel_analysis(),
        ))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use diving_core::Op;
    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Root, Size};
    use nsh_testing::{Headless, windowed_root};
    use tar::{Builder, EntryType, Header};
    use tempfile::Builder as TempBuilder;

    use super::*;
    use crate::domain::{FileTreeRow, LARGE_LAYER_BYTES};
    use crate::i18n::UiLang;
    use crate::ui::app_tokens;

    fn tar_with_entries(files: &[(&str, &[u8])], symlinks: &[(&str, &str)]) -> Vec<u8> {
        let mut builder = Builder::new(Vec::new());
        for (path, data) in files {
            let mut header = Header::new_gnu();
            header.set_path(path).expect("set tar path");
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_uid(0);
            header.set_gid(0);
            header.set_cksum();
            builder.append(&header, *data).expect("append file");
        }
        for (path, target) in symlinks {
            let mut header = Header::new_gnu();
            header.set_entry_type(EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777);
            header.set_uid(0);
            header.set_gid(0);
            header.set_link_name(target).expect("set link target");
            header.set_cksum();
            builder
                .append_link(&mut header, *path, *target)
                .expect("append symlink");
        }
        builder.into_inner().expect("finish tar")
    }

    fn docker_save_archive(layers: &[(&str, &[u8])], diff_ids: &[&str], repo_tag: &str) -> Vec<u8> {
        let history: Vec<String> = layers
            .iter()
            .enumerate()
            .map(|(idx, _)| {
                format!(
                    r#"{{"created": "2026-01-01T00:00:00Z", "created_by": "/bin/sh -c #(nop) COPY layer{idx} /"}}"#
                )
            })
            .collect();
        let diff_json: Vec<String> = diff_ids.iter().map(|id| format!(r#""{id}""#)).collect();
        let layer_names: Vec<String> = layers
            .iter()
            .map(|(name, _)| format!(r#""{name}""#))
            .collect();
        let config = format!(
            r#"{{
                "architecture": "amd64",
                "created": "2026-01-01T00:00:00Z",
                "history": [{}],
                "os": "linux",
                "rootfs": {{"type": "layers", "diff_ids": [{}]}},
                "config": {{
                    "User": "",
                    "Env": ["PATH=/usr/local/bin:/usr/bin"],
                    "Entrypoint": ["/bin/sh"],
                    "Cmd": []
                }}
            }}"#,
            history.join(","),
            diff_json.join(","),
        );
        let manifest = format!(
            r#"[{{"Config": "config.json", "RepoTags": ["{repo_tag}"], "Layers": [{}]}}]"#,
            layer_names.join(","),
        );
        let mut entries: Vec<(&str, &[u8])> = vec![
            ("manifest.json", manifest.as_bytes()),
            ("config.json", config.as_bytes()),
        ];
        entries.extend_from_slice(layers);
        tar_with_entries(&entries, &[])
    }

    fn start_headless() -> Headless {
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
            .expect("set viewport");
        app
    }

    fn write_fixture_tar(bytes: &[u8]) -> tempfile::NamedTempFile {
        let tmp = TempBuilder::new()
            .suffix(".tar")
            .tempfile()
            .expect("temp tar");
        fs::write(tmp.path(), bytes).expect("write temp tar");
        tmp
    }

    fn open_local_tar_in_app(app: &mut Headless, tar_path: &std::path::Path) {
        let image_ref = format!("file://{}", tar_path.display());
        app.type_text("Image reference input", &image_ref)
            .expect("type image ref");
        app.press("Analyze").expect("press Analyze");
        app.wait_for_within("Layers", Duration::from_secs(10))
            .expect("enter Inspector");
    }

    #[test]
    fn escape_stack_command_palette_provenance_and_file_tree_filter() {
        let layer1 = tar_with_entries(
            &[("etc/app.conf", b"listen = 8080\n")],
            &[("usr/bin/sh", "busybox")],
        );
        let archive = docker_save_archive(&[("l1/layer.tar", &layer1)], &["sha256:aaa1"], "app:v1");
        let tmp = write_fixture_tar(&archive);
        let mut app = start_headless();

        open_local_tar_in_app(&mut app, tmp.path());
        app.press("Expand all").expect("expand dirs");
        app.wait_for_within("app.conf", Duration::from_secs(3))
            .expect("show app.conf");

        // 1. Open File Preview Sheet, open Command Palette on top, search, then Esc step-by-step.
        app.press("app.conf").expect("open preview sheet");
        app.wait_for_within("Layer #1", Duration::from_secs(3))
            .expect("preview sheet open");

        app.press_key("mod+k").expect("open command palette");
        app.wait_for_within("Open local file…", Duration::from_secs(3))
            .expect("command palette open");

        app.type_text("Command Palette", "dark")
            .expect("type command query");
        app.wait_for_gone("Open local file…", Duration::from_secs(3))
            .expect("filtered to dark theme item");
        assert!(app.shows("Theme: Dark"));
        app.press_key("escape").expect("1st Esc clears query");
        app.wait_for_within("Open local file…", Duration::from_secs(3))
            .expect("query cleared restoring all items");
        assert!(
            app.shows("Layer #1"),
            "sheet should remain open after 1st Esc"
        );

        app.press_key("escape")
            .expect("2nd Esc closes only Command Palette");
        app.wait_for_gone("Open local file…", Duration::from_secs(3))
            .expect("command palette closed");
        assert!(
            app.shows("Layer #1"),
            "File Preview Sheet must remain open after closing Command Palette on top"
        );

        app.press_key("escape")
            .expect("3rd Esc closes File Preview Sheet");
        app.wait_for_gone("Layer #1", Duration::from_secs(3))
            .expect("preview sheet closed");

        // 2. Test `/` focuses filter without inserting `/` and `Esc` clears it while focused.
        app.press_key("/").expect("focus file tree filter with /");
        let _ = app.wait_for_within("Filter files in layer", Duration::from_millis(80));
        assert_ne!(
            app.screen()
                .input_text("Filter files in layer")
                .expect("read filter input"),
            Some("/"),
            "`/` shortcut must not leak a literal `/` into the filter box"
        );
        app.press_key("a").expect("type into focused filter box");
        assert_eq!(
            app.screen()
                .input_text("Filter files in layer")
                .expect("read char typed after /"),
            Some("a"),
            "filter box must stay focused and contain only `a`, never `/a`"
        );
        app.type_text("Filter files in layer", "pp.conf")
            .expect("type remaining filter keyword");
        assert_eq!(
            app.screen()
                .input_text("Filter files in layer")
                .expect("read typed filter"),
            Some("app.conf")
        );
        app.press_key("escape")
            .expect("Esc inside focused filter clears keyword");
        assert_eq!(
            app.screen()
                .input_text("Filter files in layer")
                .expect("read cleared filter"),
            Some("")
        );

        // 3. Test Provenance: 1st Esc clears focused query, 2nd Esc closes Provenance; jump resets filters.
        app.press("Modifications").expect("enable Modifications");
        app.press("Trace across layers").expect("open provenance");
        app.wait_for_within("Cross-Layer File Provenance", Duration::from_secs(3))
            .expect("provenance opened");
        app.type_text("Cross-layer file provenance search", "app.conf")
            .expect("search provenance");
        app.wait_for_within("etc/app.conf", Duration::from_secs(3))
            .expect("provenance match");
        app.press_key("escape")
            .expect("1st Esc clears focused Provenance search");
        assert!(
            app.shows("Cross-Layer File Provenance"),
            "Provenance should remain open after 1st Esc clears non-empty search text"
        );
        assert_eq!(
            app.screen()
                .input_text("Cross-layer file provenance search")
                .expect("read cleared provenance search"),
            Some("")
        );
        app.press_key("escape")
            .expect("2nd Esc closes Provenance dialog");
        app.wait_for_gone("Cross-Layer File Provenance", Duration::from_secs(3))
            .expect("provenance dismissed");

        app.press("Trace across layers")
            .expect("reopen provenance for jump test");
        app.wait_for_within("Cross-Layer File Provenance", Duration::from_secs(3))
            .expect("provenance reopened");
        app.type_text("Cross-layer file provenance search", "app.conf")
            .expect("re-type provenance query");
        app.wait_for_within("etc/app.conf", Duration::from_secs(3))
            .expect("provenance results visible");
        app.press("etc/app.conf").expect("jump to layer file");
        app.wait_for_gone("Cross-Layer File Provenance", Duration::from_secs(3))
            .expect("provenance closed after jump");
        assert!(
            app.shows("app.conf"),
            "jumped file should be visible even when Modifications was previously enabled"
        );
        assert!(!app.shows("No files match the current filter."));
    }

    #[test]
    fn layer_arrow_keys_respect_filter_and_tab_and_symlink_and_lang_relocalization() {
        let small = tar_with_entries(&[("etc/small.txt", b"small")], &[("usr/bin/sh", "busybox")]);
        let big_blob = vec![b'x'; (LARGE_LAYER_BYTES as usize) + 4096];
        let large = tar_with_entries(&[("opt/big.bin", &big_blob)], &[]);
        let tail = tar_with_entries(&[("var/tail.txt", b"tail")], &[]);

        let archive = docker_save_archive(
            &[
                ("l1/layer.tar", &small),
                ("l2/layer.tar", &large),
                ("l3/layer.tar", &tail),
            ],
            &["sha256:l1", "sha256:l2", "sha256:l3"],
            "layers:v1",
        );
        let tmp = write_fixture_tar(&archive);
        let mut app = start_headless();

        open_local_tar_in_app(&mut app, tmp.path());

        // Symlink preview shows target rather than empty.
        app.press("Expand all").expect("expand layer 1");
        app.wait_for_within("sh -> busybox", Duration::from_secs(3))
            .expect("expand usr/bin");
        app.press("sh -> busybox").expect("open symlink preview");
        app.wait_for_within("Symbolic link → busybox", Duration::from_secs(3))
            .expect("symlink target displayed");
        app.press_key("escape").expect("close symlink preview");
        app.wait_for_gone("Symbolic link → busybox", Duration::from_secs(3))
            .expect("symlink sheet closed");

        // Filter layers to > 1 MB and verify Up/Down stays on the single visible large layer.
        app.press("> 1 MB").expect("filter large layers");
        app.press_key("down")
            .expect("step down to visible large layer");
        app.wait_for_within("opt", Duration::from_secs(3))
            .expect("jumped onto layer 2 (only visible > 1 MB layer)");
        app.press_key("down").expect("down at end stays on layer 2");
        app.press_key("up").expect("up at start stays on layer 2");
        assert!(
            app.shows("opt"),
            "Up/Down must only navigate visible filtered layers"
        );

        // Switching to Findings disables Up/Down layer stepping and EN->ZH re-localizes core findings.
        app.press_key("mod+3").expect("switch to Findings");
        app.wait_for_within("Container runs as root", Duration::from_secs(3))
            .expect("EN recommendation visible");
        app.press_key("up").expect("Up on Findings must be ignored");
        app.press("中").expect("switch language to ZH");
        app.wait_for_within("容器以 root 运行", Duration::from_secs(3))
            .expect("ZH core recommendation re-localized without re-analysis");
    }

    #[test]
    fn mod_n_mod_l_failed_diff_status_toast_and_windows_uri_state() {
        let l1 = tar_with_entries(&[("etc/a.txt", b"hello")], &[]);
        let l2 = tar_with_entries(&[("etc/a.txt", b"hello"), ("opt/b.txt", b"world")], &[]);
        let base_tar = write_fixture_tar(&docker_save_archive(
            &[("l1/layer.tar", &l1)],
            &["sha256:base"],
            "demo:v1",
        ));
        let target_tar = write_fixture_tar(&docker_save_archive(
            &[("l1/layer.tar", &l2)],
            &["sha256:target"],
            "demo:v2",
        ));

        let mut app = start_headless();

        // Mod+D opens Compare dialog, Mod+N dismisses it cleanly and goes to Landing.
        app.press_key("mod+d").expect("open compare dialog");
        app.wait_for_within("Base image", Duration::from_secs(3))
            .expect("compare dialog opened");
        app.press_key("mod+n")
            .expect("Mod+N closes modal and goes home");
        app.wait_for_gone("Base image", Duration::from_secs(3))
            .expect("compare dialog closed by Mod+N");

        // Compare two valid local tars.
        app.press_key("mod+d").expect("open compare dialog");
        app.wait_for_within("Base image", Duration::from_secs(3))
            .expect("compare dialog opened");
        let base_ref = format!("file://{}", base_tar.path().display());
        let target_ref = format!("file://{}", target_tar.path().display());
        app.type_text("Base image input", &base_ref)
            .expect("fill base");
        app.type_text("Target image input", &target_ref)
            .expect("fill target");
        app.press("Compare").expect("start diff");
        app.wait_for_within("Open Base", Duration::from_secs(10))
            .expect("diff completed");

        // Mod+L on Diff opens Compare dialog instead of setting header_search_open.
        app.press_key("mod+l").expect("Mod+L on Diff");
        app.wait_for_within("Base image", Duration::from_secs(3))
            .expect("Mod+L opened Compare dialog on Diff");

        // Starting a diff that fails clears previous diff_result so Swap cannot resurrect it.
        app.type_text(
            "Base image input",
            "file:///tmp/definitely-missing-diving-base.tar",
        )
        .expect("replace base with nonexistent tar");
        app.press("Compare").expect("run failing diff");
        app.wait_for_within("Base image failed", Duration::from_secs(10))
            .expect("failed diff error visible");
        assert!(
            !app.shows("Open Base"),
            "stale Open Base action must be cleared on new/failed diff"
        );
        assert!(
            !app.shows("Swap"),
            "Swap action must not remain available from stale diff after failure"
        );

        let symlink_row = FileTreeRow {
            key: "usr/bin/sh".to_owned(),
            name: "sh".to_owned(),
            link: "busybox".to_owned(),
            mode: "lrwxrwxrwx".to_owned(),
            uid: 0,
            gid: 0,
            size: 0,
            op: Op::Added,
            depth: 2,
            is_dir: false,
            expanded: false,
            ratio: 0.0,
        };
        assert!(symlink_row.is_symlink());
        assert_eq!(UiLang::En.as_storage_str(), "en");
    }

    #[component]
    fn StateNoticeHarness() -> impl IntoElement {
        let state = AppState::new(DivingService::new());
        state.analyze("alpine:3.20".to_owned(), Some("arm64".to_owned()));
        assert_eq!(
            state
                .arch_input
                .get_untracked()
                .as_ref()
                .map(SharedString::as_str),
            Some("arm64")
        );
        state.handle_dropped_uri("file:///C:/Users/demo/image.tar");
        assert_eq!(
            state.image_input.get_untracked(),
            "file://C:/Users/demo/image.tar"
        );
        assert_eq!(state.arch_input.get_untracked(), None);
        state.cancel_analysis();

        // Simulate switching between two analyzed images and verifying Compare target refreshes.
        state.current_image.set("redis:alpine".to_owned());
        state.arch_input.set(Some(SharedString::from("amd64")));
        state.open_compare_dialog();
        assert_eq!(state.compare_target_input.get_untracked(), "redis:alpine");
        state.compare_dialog_open.set(false);
        state.current_image.set("nginx:alpine".to_owned());
        state.arch_input.set(Some(SharedString::from("arm64")));
        state.open_compare_dialog();
        assert_eq!(state.compare_target_input.get_untracked(), "nginx:alpine");
        assert_eq!(
            state
                .compare_target_arch
                .get_untracked()
                .as_ref()
                .map(SharedString::as_str),
            Some("arm64")
        );
        state.compare_dialog_open.set(false);

        state.show_status_notice("Saved to /tmp/report.md.".to_owned());

        let notice_state = state.clone();
        view().relative().w_full().h_full().child(when(
            move || notice_state.status_notice.get().is_some(),
            move || floating_status_toast(notice_state.clone()),
        ))
    }

    #[test]
    fn reactive_state_notice_and_windows_uri_and_compare_target_refresh() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(960.0), Points(640.0)),
            ..EngineConfig::new(windowed_root::<StateNoticeHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start state notice harness");
        app.wait_for_within("Saved to /tmp/report.md.", Duration::from_secs(3))
            .expect("global status notice toast rendered");
        app.press("Close").expect("dismiss floating status notice");
        app.wait_for_gone("Saved to /tmp/report.md.", Duration::from_secs(3))
            .expect("floating status notice dismissed");
    }
}
