use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};

/// The one sound playing; a new file replaces it.
struct Playing {
    _device: MixerDeviceSink,
    sink: Player,
    path: PathBuf,
    total: Option<Duration>,
}

thread_local! {
    static PLAYER: RefCell<Option<Playing>> = const { RefCell::new(None) };
    static LENGTHS: RefCell<HashMap<PathBuf, Option<Duration>>> = RefCell::new(HashMap::new());
}

/// How long a file runs, read once from its header; `None` when the format does not say.
pub fn length(path: &Path) -> Option<Duration> {
    LENGTHS.with(|l| *l.borrow_mut().entry(path.to_path_buf()).or_insert_with(|| std::fs::File::open(path).ok().and_then(|f| Decoder::try_from(f).ok()).and_then(|d| d.total_duration())))
}

/// Whether any sound is playing right now.
pub fn any_playing() -> bool {
    PLAYER.with(|p| p.borrow().as_ref().is_some_and(|x| !x.sink.is_paused() && !x.sink.empty()))
}

/// Starts, pauses or resumes a file.
pub fn toggle(path: &Path) -> Result<(), String> {
    let handled = PLAYER.with(|p| {
        let p = p.borrow();
        match p.as_ref().filter(|x| x.path == path) {
            Some(player) if player.sink.empty() => false,
            Some(player) => {
                if player.sink.is_paused() {
                    player.sink.play();
                } else {
                    player.sink.pause();
                }
                true
            }
            None => false,
        }
    });
    if handled {
        return Ok(());
    }
    let device = DeviceSinkBuilder::open_default_sink().map_err(|e| e.to_string())?;
    let sink = Player::connect_new(device.mixer());
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let source = Decoder::try_from(file).map_err(|e| e.to_string())?;
    let total = source.total_duration();
    sink.append(source);
    PLAYER.with(|p| *p.borrow_mut() = Some(Playing { _device: device, sink, path: path.to_path_buf(), total }));
    Ok(())
}

pub fn is_playing(path: &Path) -> bool {
    PLAYER.with(|p| p.borrow().as_ref().is_some_and(|x| x.path == path && !x.sink.is_paused() && !x.sink.empty()))
}

/// Where a file is and how long it runs.
pub fn progress(path: &Path) -> (Duration, Option<Duration>) {
    PLAYER.with(|p| p.borrow().as_ref().filter(|x| x.path == path).map_or((Duration::ZERO, None), |x| (x.sink.get_pos(), x.total)))
}

/// Jumps to a share of the file.
pub fn seek(path: &Path, ratio: f32) {
    PLAYER.with(|p| {
        if let Some(player) = p.borrow().as_ref().filter(|x| x.path == path)
            && let Some(total) = player.total
        {
            let _ = player.sink.try_seek(total.mul_f32(ratio.clamp(0., 1.)));
        }
    });
}
