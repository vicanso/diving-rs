//! Private registry per-domain auth, custom headers, and HTTP/TLS configuration modal.

use nsh_core::AccessibilityState;
use nsh_core::prelude::*;

use crate::domain::{AppState, RegistryDraftRule, RegistryRetryTarget};
use crate::i18n::t;
use crate::ui::{
    BadgeTone, BannerTone, ButtonVariant, IconName, UiSize, font_mono_family, radius_inset,
    surface_active, theme, ui_badge, ui_banner, ui_button, ui_checkbox, ui_dialog, ui_empty_state,
    ui_field, ui_icon, ui_icon_button, ui_input, ui_password_input, ui_separator,
};

/// Centered configuration modal managing per-domain private registry rules (`~/.diving/registries.yml`).
pub fn registry_auth_dialog(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let body_state = state.clone();
    let footer_state = state.clone();

    ui_dialog(
        state.registry_dialog_open,
        move || SharedString::from(t(lang.get(), "registry.title")),
        px(780.0),
        move || dialog_body(body_state.clone()).into_element(),
        move || dialog_footer(footer_state.clone()).into_element(),
    )
}

fn dialog_body(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let err_sig = state.registry_dialog_error;
    let list_state = state.clone();
    let editor_state = state;

    view()
        .flex_col()
        .gap_3()
        .child(
            text(move || t(lang.get(), "registry.subtitle"))
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(when(
            move || err_sig.get().is_some(),
            move || {
                ui_banner(
                    BannerTone::Danger,
                    move || SharedString::from(err_sig.get().unwrap_or_default()),
                    None,
                )
            },
        ))
        .child(
            view()
                .h(px(420.0))
                .flex_row()
                .gap_4()
                .child(rules_sidebar(list_state))
                .child(rule_editor_pane(editor_state)),
        )
}

fn rules_sidebar(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let add_state = state.clone();
    let list_state = state.clone();
    let drafts_sig = state.registry_drafts;
    let sel_sig = state.registry_selected_index;
    let live_host_sig = state.reg_host_input;

    view()
        .w(px(220.0))
        .flex_shrink_0()
        .flex_col()
        .rounded(radius_inset)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().muted.opacity(0.45))
        .overflow_hidden()
        .child(
            view()
                .px_2p5()
                .py_2()
                .flex_row()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(move || theme().border)
                .child(
                    text(move || {
                        let count = drafts_sig.with(Vec::len);
                        format!("{} ({count})", t(lang.get(), "registry.host_label"))
                    })
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().muted_foreground),
                )
                .child(ui_button(
                    move || SharedString::from(t(lang.get(), "registry.add")),
                    ButtonVariant::Outline,
                    UiSize::Xs,
                    Some(IconName::Plus),
                    false,
                    false,
                    move || add_state.add_registry_draft(),
                )),
        )
        .child(
            scroll()
                .flex_1()
                .min_h(px(0.0))
                .p_1p5()
                .child(dynamic(move || {
                    let drafts = drafts_sig.get();
                    if drafts.is_empty() {
                        return view()
                            .p_3()
                            .items_center()
                            .justify_center()
                            .child(
                                text(move || t(lang.get(), "registry.empty_title"))
                                    .text_xs()
                                    .text_color(move || theme().muted_foreground),
                            )
                            .into_element();
                    }

                    let selected = sel_sig.get();
                    let mut col = view().flex_col().gap_1();
                    for (idx, draft) in drafts.into_iter().enumerate() {
                        let is_selected = selected == Some(idx);
                        let display_host = if is_selected {
                            let live = live_host_sig.get();
                            if live.trim().is_empty() {
                                None
                            } else {
                                Some(live.trim().to_owned())
                            }
                        } else if draft.host.trim().is_empty() {
                            None
                        } else {
                            Some(draft.host.clone())
                        };
                        let click_state = list_state.clone();
                        col = col.child(rule_list_row(
                            click_state,
                            idx,
                            display_host,
                            draft,
                            is_selected,
                            lang,
                        ));
                    }
                    col.into_element()
                })),
        )
}

fn rule_list_row(
    state: AppState,
    idx: usize,
    display_host: Option<String>,
    draft: RegistryDraftRule,
    is_selected: bool,
    lang: Signal<crate::i18n::UiLang>,
) -> impl IntoElement {
    let press_state = state.clone();
    let label_for_a11y = display_host
        .clone()
        .unwrap_or_else(|| t(lang.get_untracked(), "registry.unnamed").to_owned());

    view()
        .w_full()
        .px_2()
        .py_1p5()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_1p5()
        .rounded(radius_inset)
        .role(AccessibilityRole::Button)
        .label(label_for_a11y)
        .accessibility_state(move || AccessibilityState {
            selected: is_selected,
            ..AccessibilityState::default()
        })
        .focusable(true)
        .bg(move || {
            if is_selected {
                surface_active()
            } else {
                AdaptiveColor::from(transparent_black())
            }
        })
        .hover(move |s| {
            if is_selected {
                s.bg(surface_active)
            } else {
                s.bg(move || theme().accent)
            }
        })
        .on_press(move || press_state.select_registry_draft(idx))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(ui_icon(IconName::FileKey, px(13.0), move || {
                    if is_selected {
                        theme().primary
                    } else {
                        theme().muted_foreground
                    }
                }))
                .child(
                    text({
                        let value = display_host.clone();
                        move || {
                            value
                                .clone()
                                .unwrap_or_else(|| t(lang.get(), "registry.unnamed").to_owned())
                        }
                    })
                    .flex_1()
                    .min_w(px(0.0))
                    .text_xs()
                    .font_family(font_mono_family())
                    .number_of_lines(1)
                    .text_color(move || {
                        if display_host.is_some() {
                            theme().foreground
                        } else {
                            theme().muted_foreground
                        }
                    }),
                ),
        )
        .child(when(draft.from_config_yml, move || {
            ui_badge(
                move || SharedString::from(t(lang.get(), "registry.badge_config_yml")),
                BadgeTone::Neutral,
            )
        }))
}

fn rule_editor_pane(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let sel_sig = state.registry_selected_index;
    let drafts_sig = state.registry_drafts;

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex_col()
        .child(dynamic(move || {
            let Some(idx) = sel_sig.get() else {
                let add_state = state.clone();
                return view()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .child(ui_empty_state(
                        IconName::FileKey,
                        move || SharedString::from(t(lang.get(), "registry.empty_title")),
                        move || SharedString::from(t(lang.get(), "registry.empty_desc")),
                        Some(
                            ui_button(
                                move || SharedString::from(t(lang.get(), "registry.add")),
                                ButtonVariant::Primary,
                                UiSize::Sm,
                                Some(IconName::Plus),
                                false,
                                false,
                                move || add_state.add_registry_draft(),
                            )
                            .into_element(),
                        ),
                    ))
                    .into_element();
            };

            let from_config =
                drafts_sig.with(|list| list.get(idx).is_some_and(|rule| rule.from_config_yml));
            let reset_state = state.clone();
            let delete_state = state.clone();
            let add_header_state = state.clone();
            let browse_state = state.clone();
            let headers_state = state.clone();
            let each_headers_sig = state.reg_headers;
            let each_rm_state = state.clone();
            let insecure_http_sig = state.reg_insecure_http;
            let skip_tls_sig = state.reg_skip_tls;

            scroll()
                .flex_1()
                .min_h(px(0.0))
                .child(
                    view()
                        .pr_2()
                        .pb_2()
                        .flex_col()
                        .gap_3()
                        .child(ui_field(
                            move || SharedString::from(t(lang.get(), "registry.host_label")),
                            Some(SharedString::from(t(
                                lang.get_untracked(),
                                "registry.host_help",
                            ))),
                            || None::<SharedString>,
                            ui_input(
                                state.reg_host_input,
                                move || t(lang.get(), "registry.host_placeholder").to_owned(),
                                "Registry host or wildcard pattern",
                                UiSize::Md,
                                true,
                                None,
                            ),
                        ))
                        .child(
                            view()
                                .flex_row()
                                .items_start()
                                .gap_3()
                                .child(view().flex_1().min_w(px(0.0)).child(ui_field(
                                    move || {
                                        SharedString::from(t(lang.get(), "registry.username_label"))
                                    },
                                    None,
                                    || None::<SharedString>,
                                    ui_input(
                                        state.reg_username_input,
                                        move || {
                                            t(lang.get(), "registry.username_placeholder")
                                                .to_owned()
                                        },
                                        "Registry username",
                                        UiSize::Md,
                                        true,
                                        None,
                                    ),
                                )))
                                .child(view().flex_1().min_w(px(0.0)).child(ui_field(
                                    move || {
                                        SharedString::from(t(lang.get(), "registry.password_label"))
                                    },
                                    None,
                                    || None::<SharedString>,
                                    ui_password_input(
                                        state.reg_password_input,
                                        move || {
                                            t(lang.get(), "registry.password_placeholder")
                                                .to_owned()
                                        },
                                        "Registry password",
                                        UiSize::Md,
                                        true,
                                    ),
                                ))),
                        )
                        .child(ui_field(
                            move || SharedString::from(t(lang.get(), "registry.token_label")),
                            Some(SharedString::from(t(
                                lang.get_untracked(),
                                "registry.token_help",
                            ))),
                            || None::<SharedString>,
                            ui_password_input(
                                state.reg_token_input,
                                move || t(lang.get(), "registry.token_placeholder").to_owned(),
                                "Static Bearer token",
                                UiSize::Md,
                                true,
                            ),
                        ))
                        .child(ui_separator())
                        .child(
                            view()
                                .flex_col()
                                .gap_2()
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            text(move || t(lang.get(), "registry.headers_label"))
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(move || theme().foreground),
                                        )
                                        .child(ui_button(
                                            move || {
                                                SharedString::from(t(
                                                    lang.get(),
                                                    "registry.headers_add",
                                                ))
                                            },
                                            ButtonVariant::Outline,
                                            UiSize::Xs,
                                            Some(IconName::Plus),
                                            false,
                                            false,
                                            move || add_header_state.add_registry_header_row(),
                                        )),
                                )
                                .child(when(
                                    move || headers_state.reg_headers.with(Vec::is_empty),
                                    move || {
                                        text(move || t(lang.get(), "registry.headers_empty"))
                                            .text_xs()
                                            .text_color(move || theme().muted_foreground)
                                    },
                                ))
                                .child(each(
                                    move || each_headers_sig.get(),
                                    |row| row.id,
                                    move |row_sig, _| {
                                        let rm_state = each_rm_state.clone();
                                        let row = row_sig.get_untracked();
                                        let row_id = row.id;
                                        view()
                                            .flex_row()
                                            .items_center()
                                            .gap_2()
                                            .child(view().flex_1().min_w(px(0.0)).child(ui_input(
                                                row.key,
                                                move || {
                                                    t(
                                                        lang.get(),
                                                        "registry.header_name_placeholder",
                                                    )
                                                    .to_owned()
                                                },
                                                "Custom header name",
                                                UiSize::Sm,
                                                true,
                                                None,
                                            )))
                                            .child(view().flex_1().min_w(px(0.0)).child(
                                                ui_password_input(
                                                    row.value,
                                                    move || {
                                                        t(
                                                            lang.get(),
                                                            "registry.header_value_placeholder",
                                                        )
                                                        .to_owned()
                                                    },
                                                    "Custom header value",
                                                    UiSize::Sm,
                                                    true,
                                                ),
                                            ))
                                            .child(ui_icon_button(
                                                IconName::Close,
                                                "Remove custom header",
                                                Some(move || {
                                                    SharedString::from(t(
                                                        lang.get(),
                                                        "common.close",
                                                    ))
                                                }),
                                                ButtonVariant::Ghost,
                                                UiSize::Sm,
                                                false,
                                                false,
                                                move || rm_state.remove_registry_header_row(row_id),
                                            ))
                                    },
                                )),
                        )
                        .child(ui_separator())
                        .child(
                            view()
                                .flex_col()
                                .gap_2()
                                .child(ui_checkbox(
                                    move || {
                                        SharedString::from(t(lang.get(), "registry.insecure_http"))
                                    },
                                    insecure_http_sig,
                                    false,
                                    false,
                                    move |next| insecure_http_sig.set(next),
                                ))
                                .child(ui_checkbox(
                                    move || SharedString::from(t(lang.get(), "registry.skip_tls")),
                                    skip_tls_sig,
                                    false,
                                    false,
                                    move |next| skip_tls_sig.set(next),
                                ))
                                .child(ui_field(
                                    move || {
                                        SharedString::from(t(lang.get(), "registry.ca_cert_label"))
                                    },
                                    None,
                                    || None::<SharedString>,
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .child(view().flex_1().min_w(px(0.0)).child(ui_input(
                                            state.reg_ca_cert_input,
                                            move || {
                                                t(lang.get(), "registry.ca_cert_placeholder")
                                                    .to_owned()
                                            },
                                            "CA certificate path",
                                            UiSize::Sm,
                                            true,
                                            None,
                                        )))
                                        .child(ui_button(
                                            move || {
                                                SharedString::from(t(
                                                    lang.get(),
                                                    "registry.ca_cert_browse",
                                                ))
                                            },
                                            ButtonVariant::Outline,
                                            UiSize::Sm,
                                            Some(IconName::FolderOpen),
                                            false,
                                            false,
                                            move || browse_state.browse_registry_ca_cert(),
                                        )),
                                )),
                        )
                        .child(
                            view()
                                .pt_1()
                                .flex_row()
                                .items_center()
                                .justify_end()
                                .gap_2()
                                .when(from_config, |row| {
                                    row.child(ui_button(
                                        move || {
                                            SharedString::from(t(lang.get(), "registry.reset_rule"))
                                        },
                                        ButtonVariant::Outline,
                                        UiSize::Sm,
                                        Some(IconName::Refresh),
                                        false,
                                        false,
                                        move || reset_state.reset_selected_registry_draft(),
                                    ))
                                })
                                .when(!from_config, |row| {
                                    row.child(ui_button(
                                        move || {
                                            SharedString::from(t(
                                                lang.get(),
                                                "registry.delete_rule",
                                            ))
                                        },
                                        ButtonVariant::Danger,
                                        UiSize::Sm,
                                        Some(IconName::Close),
                                        false,
                                        false,
                                        move || delete_state.remove_selected_registry_draft(),
                                    ))
                                }),
                        ),
                )
                .into_element()
        }))
}

fn dialog_footer(state: AppState) -> impl IntoElement {
    let lang = state.lang;
    let cancel_state = state.clone();
    let save_state = state.clone();
    let retry_sig = state.registry_retry_target;

    view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            text(move || t(lang.get(), "registry.footer_hint"))
                .flex_1()
                .min_w(px(0.0))
                .text_xs()
                .font_family(font_mono_family())
                .number_of_lines(1)
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .flex_shrink_0()
                .child(ui_button(
                    move || SharedString::from(t(lang.get(), "landing.cancel")),
                    ButtonVariant::Ghost,
                    UiSize::Md,
                    None,
                    false,
                    false,
                    move || cancel_state.registry_dialog_open.set(false),
                ))
                .child(ui_button(
                    move || {
                        let key = if retry_sig.get() == RegistryRetryTarget::None {
                            "registry.save"
                        } else {
                            "registry.save_and_retry"
                        };
                        SharedString::from(t(lang.get(), key))
                    },
                    ButtonVariant::Primary,
                    UiSize::Md,
                    Some(IconName::Check),
                    false,
                    false,
                    move || save_state.save_registry_settings(),
                )),
        )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use diving_core::{Analyzer, sha256_hex};
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Size};
    use nsh_testing::{Headless, windowed_root};
    use serde_json::json;
    use tar::{Builder as TarBuilder, Header as TarHeader};
    use tempfile::TempDir;

    use super::*;
    use crate::domain::{AppRoute, DiffErrorState, DivingService};
    use crate::features::{diff_workspace, inspector_workspace, landing_view};
    use crate::ui::{app_tokens, register_fonts};

    const MANIFEST_TYPE: &str = "application/vnd.docker.distribution.manifest.v2+json";

    type HarnessConfig = (PathBuf, AppRoute, Option<(String, String, String)>);

    static HARNESS_LOCK: Mutex<()> = Mutex::new(());
    static PENDING_HARNESS_CONFIG: Mutex<Option<HarnessConfig>> = Mutex::new(None);

    struct MockRegistryFixture {
        manifest: Vec<u8>,
        manifest_digest: String,
        blobs: HashMap<String, Vec<u8>>,
    }

    fn build_mock_fixture() -> MockRegistryFixture {
        let mut tar_buf = TarBuilder::new(Vec::new());
        let data = b"app_env=staging\n";
        let mut hdr = TarHeader::new_gnu();
        hdr.set_path("etc/app.conf").unwrap();
        hdr.set_size(data.len() as u64);
        hdr.set_mode(0o644);
        hdr.set_uid(0);
        hdr.set_gid(0);
        hdr.set_cksum();
        tar_buf.append(&hdr, &data[..]).unwrap();
        let tar_bytes = tar_buf.into_inner().unwrap();

        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&tar_bytes).unwrap();
        let layer = gz.finish().unwrap();

        let config = br#"{
            "architecture": "amd64",
            "created": "2025-01-01T00:00:00Z",
            "history": [{"created": "2025-01-01T00:00:00Z", "created_by": "/bin/sh -c #(nop) COPY . /"}],
            "os": "linux",
            "rootfs": {"type": "layers", "diff_ids": ["sha256:aaa"]},
            "config": {"User": "", "Env": ["PATH=/usr/bin"], "Entrypoint": ["/bin/sh"], "Cmd": []}
        }"#
        .to_vec();

        let config_digest = format!("sha256:{}", sha256_hex(&config));
        let layer_digest = format!("sha256:{}", sha256_hex(&layer));
        let manifest = serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "mediaType": MANIFEST_TYPE,
            "config": {
                "mediaType": "application/vnd.docker.container.image.v1+json",
                "size": config.len(),
                "digest": config_digest,
            },
            "layers": [{
                "mediaType": "application/vnd.docker.image.rootfs.diff.tar.gzip",
                "size": layer.len(),
                "digest": layer_digest,
            }],
        }))
        .unwrap();
        let manifest_digest = format!("sha256:{}", sha256_hex(&manifest));

        MockRegistryFixture {
            manifest,
            manifest_digest,
            blobs: HashMap::from([(config_digest, config), (layer_digest, layer)]),
        }
    }

    struct MockAuthRegistry {
        addr: SocketAddr,
        unauth_hits: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        handle: Option<JoinHandle<()>>,
    }

    impl MockAuthRegistry {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock auth registry");
            listener.set_nonblocking(true).expect("set nonblocking");
            let addr = listener.local_addr().expect("mock local_addr");
            let fixture = Arc::new(build_mock_fixture());
            let unauth_hits = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));

            let thread_hits = unauth_hits.clone();
            let thread_stop = stop.clone();
            let handle = thread::spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            handle_mock_connection(stream, &fixture, &thread_hits);
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });

            Self {
                addr,
                unauth_hits,
                stop,
                handle: Some(handle),
            }
        }
    }

    impl Drop for MockAuthRegistry {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn write_http_response(
        stream: &mut TcpStream,
        status: &str,
        extra_headers: &[(&str, &str)],
        body: &[u8],
    ) {
        let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
        for (k, v) in extra_headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("Connection: close\r\n\r\n");
        let _ = stream.write_all(head.as_bytes());
        if !body.is_empty() {
            let _ = stream.write_all(body);
        }
        let _ = stream.flush();
    }

    fn handle_mock_connection(
        mut stream: TcpStream,
        fixture: &MockRegistryFixture,
        unauth_hits: &AtomicUsize,
    ) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut reader = BufReader::new(stream.try_clone().expect("clone tcp stream"));
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let target = parts.next().unwrap_or("/");
        let path = target.split('?').next().unwrap_or("/");

        let mut headers = HashMap::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some((k, v)) = trimmed.split_once(':') {
                headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
            }
        }

        // Expect `Authorization: Basic base64("user:pass")` and custom gateway header.
        let auth_ok =
            headers.get("authorization").map(String::as_str) == Some("Basic dXNlcjpwYXNz");
        let gw_ok = headers.get("x-custom-gw").map(String::as_str) == Some("gw-ok");
        if !auth_ok || !gw_ok {
            unauth_hits.fetch_add(1, Ordering::SeqCst);
            write_http_response(
                &mut stream,
                "401 Unauthorized",
                &[("WWW-Authenticate", "Basic realm=\"Mock Test Registry\"")],
                b"",
            );
            return;
        }

        if path == "/v2/team/app/manifests/latest" {
            let body = if method == "HEAD" {
                &[][..]
            } else {
                fixture.manifest.as_slice()
            };
            write_http_response(
                &mut stream,
                "200 OK",
                &[
                    ("Content-Type", MANIFEST_TYPE),
                    ("Docker-Content-Digest", &fixture.manifest_digest),
                ],
                body,
            );
            return;
        }

        if let Some(digest) = path.strip_prefix("/v2/team/app/blobs/")
            && let Some(blob) = fixture.blobs.get(digest)
        {
            let body = if method == "HEAD" {
                &[][..]
            } else {
                blob.as_slice()
            };
            write_http_response(&mut stream, "200 OK", &[], body);
            return;
        }

        write_http_response(&mut stream, "404 Not Found", &[], b"");
    }

    #[component]
    fn RegistryAuthFlowHarness() -> impl IntoElement {
        register_fonts();
        let (config_dir, initial_route, initial_diff_err) = PENDING_HARNESS_CONFIG
            .lock()
            .unwrap()
            .take()
            .expect("PENDING_HARNESS_CONFIG initialized before Headless start");
        let analyzer = Analyzer::builder()
            .allow_local_sources(true)
            .build()
            .expect("build isolated analyzer");
        let service = DivingService::with_analyzer_and_config_dir(analyzer, config_dir);
        let state = AppState::new(service);
        state.route.set(initial_route);
        if let Some((base_ref, target_ref, err_msg)) = initial_diff_err {
            state.diff_base_ref.set(base_ref);
            state.diff_target_ref.set(target_ref);
            state.diff_error.set(Some(DiffErrorState {
                base: Some(err_msg),
                target: None,
            }));
        }
        provide_context(state.clone());

        let route_state = state.clone();
        view()
            .flex_1()
            .flex_col()
            .bg(move || theme().background)
            .text_color(move || theme().foreground)
            .child(dynamic(move || match route_state.route.get() {
                AppRoute::Landing => landing_view(route_state.clone()).into_element(),
                AppRoute::Inspector => inspector_workspace(route_state.clone()).into_element(),
                AppRoute::Diff => diff_workspace(route_state.clone()).into_element(),
                #[cfg(debug_assertions)]
                AppRoute::Gallery => view().into_element(),
            }))
            .child(registry_auth_dialog(state))
    }

    fn start_harness(
        config_dir: PathBuf,
        route: AppRoute,
        diff_err: Option<(String, String, String)>,
    ) -> Headless {
        let _guard = HARNESS_LOCK.lock().unwrap();
        *PENDING_HARNESS_CONFIG.lock().unwrap() = Some((config_dir, route, diff_err));
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(1280.0), Points(820.0)),
            ..EngineConfig::new(windowed_root::<RegistryAuthFlowHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start registry auth headless harness");
        app.set_viewport(Size::new(Points(1280.0), Points(820.0)))
            .expect("set desktop viewport");
        app
    }

    #[test]
    fn failed_auth_banner_opens_prefilled_modal_saves_0600_and_retries_analysis() {
        let tmp = TempDir::new().expect("create isolated config temp dir");
        let mock = MockAuthRegistry::start();
        let host = mock.addr.to_string();
        let image_ref = format!("http://{host}/team/app:latest");

        let mut app = start_harness(tmp.path().to_path_buf(), AppRoute::Landing, None);

        app.type_text("Image reference input", &image_ref)
            .expect("enter private image ref");
        app.press("Analyze").expect("press Analyze");

        app.wait_for_within("Configure Auth…", Duration::from_secs(5))
            .expect("show Configure Auth action button on 401 banner");
        assert!(
            mock.unauth_hits.load(Ordering::SeqCst) >= 1,
            "expected initial unauthenticated request to hit mock registry"
        );

        app.press("Configure Auth…")
            .expect("open registry modal from error banner");
        app.wait_for_within("Private Registries", Duration::from_secs(3))
            .expect("open Private Registries dialog");
        app.wait_for_within("Save & Retry", Duration::from_secs(2))
            .expect("show Save & Retry button when opened from error banner");
        {
            let screen = app.screen();
            assert_eq!(
                screen.input_text("Registry host or wildcard pattern"),
                Ok(Some(host.as_str()))
            );
        }

        app.type_text("Registry username", "user")
            .expect("enter username");
        app.type_text("Registry password", "pass")
            .expect("enter password");
        app.press("Add Header").expect("add custom header row");
        app.wait_for_within("Custom header name", Duration::from_secs(2))
            .expect("mount custom header key input");
        app.type_text("Custom header name", "X-Custom-Gw")
            .expect("enter header name");
        app.type_text("Custom header value", "gw-ok")
            .expect("enter header value");
        app.press("Default to plain HTTP (http://) instead of HTTPS")
            .expect("toggle plain HTTP checkbox");
        app.press("Save & Retry").expect("save and retry analysis");

        app.wait_for_within("Efficiency", Duration::from_secs(10))
            .expect("hot-reloaded analyzer should complete retry into Inspector");

        let reg_file = tmp.path().join("registries.yml");
        assert!(reg_file.is_file(), "expected registries.yml to be written");
        #[cfg(unix)]
        {
            let mode = fs::metadata(&reg_file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let saved = fs::read_to_string(&reg_file).unwrap();
        assert!(
            saved.contains(&host),
            "saved yaml should include {host}: {saved}"
        );
        assert!(
            saved.contains("X-Custom-Gw"),
            "saved yaml should include custom header: {saved}"
        );
        assert!(
            saved.contains("insecure_http: true"),
            "saved yaml should include insecure_http: {saved}"
        );
    }

    #[test]
    fn diff_error_auth_banner_opens_registry_modal_with_save_and_retry() {
        let tmp = TempDir::new().expect("create isolated config temp dir");
        let host = "harbor.corp.internal:8443".to_owned();
        let mut app = start_harness(
            tmp.path().to_path_buf(),
            AppRoute::Diff,
            Some((
                format!("{host}/team/app:v1"),
                format!("{host}/team/app:v2"),
                "HTTP 401 Unauthorized: authentication required".to_owned(),
            )),
        );

        app.wait_for_within("Configure Auth…", Duration::from_secs(3))
            .expect("show Configure Auth button on diff error card");
        app.press("Configure Auth…")
            .expect("open registry auth modal from diff error");
        app.wait_for_within("Private Registries", Duration::from_secs(3))
            .expect("open Private Registries dialog from diff error");
        app.wait_for_within("Save & Retry", Duration::from_secs(2))
            .expect("show Save & Retry button for Diff retry target");
        {
            let screen = app.screen();
            assert_eq!(
                screen.input_text("Registry host or wildcard pattern"),
                Ok(Some(host.as_str()))
            );
        }
    }
}
