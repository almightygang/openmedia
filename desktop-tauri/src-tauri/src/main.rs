mod anilist;
mod cmd;
mod providers;
mod proxy;
mod store;

use std::sync::Mutex;
use tauri::{Emitter, Manager};

pub struct ProxyPort(pub Mutex<u16>);

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // Forward deep-link args (e.g. openmedia://) to the running instance.
            if let Some(url) = argv.iter().find(|a| a.starts_with("openmedia://")) {
                let _ = app.emit("deep-link", url.clone());
                let st = app.state::<anilist::AniListState>();
                anilist::oauth::complete_oauth_from_deep_link(app, &st, url);
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(ProxyPort(Mutex::new(0)))
        .manage(anilist::AniListState::new())
        .manage(cmd::providers::build_providers())
        .setup(|app| {
            // Register openmedia:// protocol for AniList OAuth callback.
            #[cfg(desktop)]
            {
                use tauri_plugin_deep_link::DeepLinkExt as _;
                let _ = app.deep_link().register("openmedia");
                // Complete a pending AniList sign-in from the callback URL.
                let handle = app.handle().clone();
                let _ = app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
                        let st = handle.state::<anilist::AniListState>();
                        anilist::oauth::complete_oauth_from_deep_link(&handle, &st, url.as_str());
                    }
                });
                // Cold start via deep link (no pending handshake → no-op, like upstream).
                if let Ok(Some(urls)) = app.deep_link().get_current() {
                    for url in urls {
                        let st = app.handle().state::<anilist::AniListState>();
                        anilist::oauth::complete_oauth_from_deep_link(
                            app.handle(),
                            &st,
                            url.as_str(),
                        );
                    }
                }
            }
            // Resolve ffmpeg sidecar, publish proxy state, start local server.
            let handle = app.handle().clone();
            let ffmpeg = proxy::ffmpeg::resolve_ffmpeg_path(&handle);
            handle.manage(proxy::ProxyState::with_ffmpeg(ffmpeg));
            let state = handle.state::<proxy::ProxyState>().inner().clone();
            tauri::async_runtime::spawn(async move {
                let port = proxy::server::serve(state).await;
                *handle.state::<ProxyPort>().0.lock().unwrap() = port;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(
                event,
                tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
            ) {
                let state = window.state::<proxy::ProxyState>().inner().clone();
                proxy::transcode::shutdown_transcode_jobs(&state);
            }
        })
        .invoke_handler(tauri::generate_handler![
            // app
            cmd::app::app_version,
            cmd::app::app_os,
            cmd::app::app_check_for_update,
            cmd::app::app_open_logs_directory,
            cmd::app::app_list_github_issues,
            cmd::app::app_list_pinned_github_issues,
            // theme
            cmd::theme::theme_current,
            cmd::theme::theme_toggle,
            cmd::theme::theme_dark,
            cmd::theme::theme_light,
            cmd::theme::theme_system,
            // window
            cmd::window::window_minimize,
            cmd::window::window_close,
            cmd::window::window_toggle_maximize,
            cmd::window::window_is_maximized,
            // external
            cmd::external::external_open_url,
            // privacy
            cmd::privacy::privacy_incognito_get,
            cmd::privacy::privacy_incognito_set,
            // recently watched
            cmd::history::recently_watched_upsert,
            cmd::history::recently_watched_read,
            cmd::history::recently_watched_clear,
            cmd::history::recently_watched_remove,
            // stream provider
            cmd::providers::stream_provider_active_get,
            cmd::providers::stream_provider_active_set,
            cmd::providers::stream_provider_search,
            cmd::providers::stream_provider_search_filters,
            cmd::providers::stream_provider_episodes,
            cmd::providers::stream_provider_stream_url,
            cmd::providers::stream_provider_proxy_base,
            cmd::providers::stream_provider_prepare_transcode,
            cmd::providers::stream_provider_cancel_transcode,
            cmd::providers::stream_provider_transcode_progress,
            cmd::providers::stream_provider_show_details,
            cmd::providers::stream_provider_recent_uploads,
            // anilist
            cmd::anilist::anilist_show_details,
            cmd::anilist::anilist_search,
            cmd::anilist::anilist_popular_season,
            cmd::anilist::anilist_connect,
            cmd::anilist::anilist_disconnect,
            cmd::anilist::anilist_status,
            cmd::anilist::anilist_open_pin_auth,
            cmd::anilist::anilist_submit_manual_token,
            cmd::anilist::anilist_get_media_list,
            cmd::anilist::anilist_get_favourites,
            cmd::anilist::anilist_save_list_entry,
            cmd::anilist::anilist_delete_list_entry,
            cmd::anilist::anilist_toggle_favourite,
            cmd::anilist::anilist_sync_watch_progress,
        ])
        .run(tauri::generate_context!())
        .expect("error while running openmedia");
}
