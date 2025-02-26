mod uploader;

use eyre::{eyre, Context};
use notify::{
    event::{AccessKind, AccessMode},
    EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::mpsc,
};

fn main() {
    tracing_subscriber::fmt::init();
    color_eyre::install().expect("cannot install color eyre");

    let ignore_dotfiles = env::var("IGNORE_DOTFILES")
        .map(|v| {
            v.parse::<bool>()
                .expect("invalid IGNORE_DOTFILES env variable")
        })
        .unwrap_or(true);

    let watch_dir: PathBuf = env::var_os("WATCH_DIR")
        .expect("WATCH_DIR not provided")
        .into();

    let watch_dir = fs::canonicalize(watch_dir).expect("cannot canonicalize path");

    let (fs_event_tx, fs_event_rx) = mpsc::channel();

    let mut watcher = notify::recommended_watcher(fs_event_tx).expect("cannot create watcher");

    watcher
        .watch(&watch_dir, RecursiveMode::Recursive)
        .expect("cannot watch directory");

    tracing::info!(
        "watching {} using {:?}",
        watch_dir.display(),
        RecommendedWatcher::kind()
    );

    let discord_webhook_url = env::var("WEBHOOK_URL").expect("no WEBHOOK_URL");

    let uploader = uploader::Discord::new(discord_webhook_url);

    for res in fs_event_rx {
        if let Err(error) = handle_event(res, &watch_dir, ignore_dotfiles, uploader.clone()) {
            tracing::error!("error while handling event: {error:?}");
        }
    }
}

fn handle_event(
    event: Result<notify::Event, notify::Error>,
    watch_dir: &Path,
    ignore_dotfiles: bool,
    uploader: uploader::Discord,
) -> eyre::Result<()> {
    let event = event.wrap_err("error in event")?;

    // check if the event is a close event on a writable file
    if matches!(
        event.kind,
        EventKind::Access(AccessKind::Close(AccessMode::Write))
    ) {
        let full_path = event.paths.first().ok_or(eyre!("no path in fs event"))?;

        let relative_path = pathdiff::diff_paths(full_path, watch_dir)
            .ok_or(eyre!("cannot get relative path of modified file"))?;

        let parent_directory = relative_path
            .parent()
            .ok_or(eyre!("no parent folder to modified file"))?
            .to_str()
            .ok_or(eyre!("invalid utf-8 parent folder name"))?;

        let filename = relative_path
            .file_name()
            .ok_or(eyre!("modified file has no filename"))?
            .to_str()
            .ok_or(eyre!("invalid utf-8 filename"))?;

        tracing::debug!("{relative_path:?} was modified, parent folder is '{parent_directory}'");

        if ignore_dotfiles && filename.starts_with('.') {
            tracing::debug!("file is a dotfile, ignoring");
        } else {
            uploader
                .upload(parent_directory, filename, full_path)
                .wrap_err_with(|| format!("could not upload file '{}'", full_path.display()))?;

            tracing::debug!("file uploaded!");
        }
    }

    Ok(())
}
