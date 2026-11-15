//! Превью для библиотеки: отдельный поток просит mpv (`vo=image`) снять
//! один кадр, уменьшенный до 480 точек по ширине.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::library::Kind;
use crate::mpv::{self, Player};

pub struct Job {
    pub id: String,
    pub file: PathBuf,
    pub kind: Kind,
}

pub struct Done {
    pub id: String,
    pub thumb: Option<PathBuf>,
}

pub struct Thumbnailer {
    tx: Sender<Job>,
    pub rx: Receiver<Done>,
}

impl Thumbnailer {
    pub fn start(dir: PathBuf, wake: impl Fn() + Send + 'static) -> Thumbnailer {
        let (tx, jobs) = channel::<Job>();
        let (done, rx) = channel::<Done>();
        std::thread::spawn(move || {
            for job in jobs {
                let thumb = make(&dir, &job).ok();
                let _ = done.send(Done { id: job.id, thumb });
                wake();
            }
        });
        Thumbnailer { tx, rx }
    }

    pub fn request(&self, job: Job) {
        let _ = self.tx.send(job);
    }
}

fn make(dir: &Path, job: &Job) -> Result<PathBuf, String> {
    if job.kind == Kind::Audio {
        return Err("у звука нет кадра".into());
    }
    mpv::available()?;
    let out_dir = dir.join(format!("tmp-{}", job.id));
    let _ = fs::remove_dir_all(&out_dir);
    fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let out = out_dir.to_string_lossy().into_owned();
    let start = if job.kind == Kind::Video { "20%" } else { "0" };
    let player = Player::new(&[
        ("config", "no"),
        ("load-scripts", "no"),
        ("ytdl", "no"),
        ("terminal", "no"),
        ("vo", "image"),
        ("vo-image-format", "jpg"),
        ("vo-image-jpeg-quality", "85"),
        ("vo-image-outdir", &out),
        ("ao", "null"),
        ("aid", "no"),
        ("sid", "no"),
        ("frames", "1"),
        ("start", start),
        ("hr-seek", "yes"),
        ("vf", "scale=w=480:h=-2"),
        ("image-display-duration", "0"),
    ])?;
    player.load(&job.file.to_string_lossy())?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match player.wait_event(0.5) {
            Some((mpv::EVENT_END_FILE, _)) | Some((mpv::EVENT_SHUTDOWN, _)) => break,
            _ => {}
        }
    }
    drop(player);
    let shot = fs::read_dir(&out_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|x| x == "jpg"));
    let result = match shot {
        Some(shot) => {
            let target = dir.join(format!("{}.jpg", job.id));
            fs::rename(&shot, &target).map_err(|e| e.to_string())?;
            Ok(target)
        }
        None => Err("mpv не сохранил кадр".into()),
    };
    let _ = fs::remove_dir_all(&out_dir);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Нужен настоящий libmpv; без него тест молча пропускается.
    #[test]
    fn makes_thumbnail_from_video() {
        if mpv::available().is_err() {
            eprintln!("libmpv нет — пропуск");
            return;
        }
        let dir = std::env::temp_dir().join(format!("vibewall-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let job = Job {
            id: "t1".into(),
            file: PathBuf::from("av://lavfi:testsrc=duration=5:size=1280x720:rate=25"),
            kind: Kind::Video,
        };
        let thumb = make(&dir, &job).expect("превью");
        let bytes = std::fs::read(&thumb).unwrap();
        assert!(bytes.starts_with(&[0xff, 0xd8]), "это JPEG");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
