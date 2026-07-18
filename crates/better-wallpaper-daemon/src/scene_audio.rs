use std::{io::Write, path::Path};

use better_wallpaper_scene_format::{PkgReader, SceneGraph, SceneNodeKind, SoundPlaybackMode};
use tempfile::{Builder, NamedTempFile};
use tracing::{info, warn};

use crate::playback::AudioPlayback;

const MAX_BACKGROUND_TRACKS: usize = 1;

#[derive(Debug)]
struct PreparedAudioTrack {
    file: NamedTempFile,
    resource: String,
    loop_playback: bool,
    volume: f32,
}

/// Package-backed scene audio kept alive for the lifetime of a scene runtime.
///
/// FFmpeg currently opens paths, so package entries are copied to private
/// temporary files. `NamedTempFile` removes them when the scene is dropped.
#[derive(Debug, Default)]
pub struct PreparedSceneAudio {
    tracks: Vec<PreparedAudioTrack>,
    start_silent_tracks: usize,
}

pub struct SceneAudioRuntime {
    playbacks: Vec<AudioPlayback>,
}

impl SceneAudioRuntime {
    pub fn set_paused(&mut self, paused: bool) {
        for playback in &mut self.playbacks {
            playback.set_paused(paused);
        }
    }

    pub fn active_track_count(&self) -> usize {
        self.playbacks.len()
    }
}

impl Drop for SceneAudioRuntime {
    fn drop(&mut self) {
        if !self.playbacks.is_empty() {
            info!(
                track_count = self.playbacks.len(),
                "scene background music stopped and resources released"
            );
        }
    }
}

impl PreparedSceneAudio {
    pub fn prepare(package: &PkgReader, graph: &SceneGraph) -> Self {
        let mut prepared = Self::default();
        for node in &graph.nodes {
            let SceneNodeKind::Sound(sound) = &node.kind else {
                continue;
            };
            if !node.visible {
                continue;
            }
            if sound.start_silent {
                prepared.start_silent_tracks += 1;
                info!(node = %node.id, resource = %sound.resource, "scene sound starts silent and requires an unsupported script trigger");
                continue;
            }
            if prepared.tracks.len() >= MAX_BACKGROUND_TRACKS {
                warn!(node = %node.id, resource = %sound.resource, limit = MAX_BACKGROUND_TRACKS, "scene background track limit reached; skipping additional sound");
                continue;
            }
            let Some(entry) = package.find(&sound.resource) else {
                warn!(node = %node.id, resource = %sound.resource, "scene sound resource is missing; continuing without this track");
                continue;
            };
            let suffix = safe_audio_suffix(&sound.resource);
            let mut file = match Builder::new()
                .prefix("better-wallpaper-scene-audio-")
                .suffix(&suffix)
                .tempfile()
            {
                Ok(file) => file,
                Err(error) => {
                    warn!(%error, resource = %sound.resource, "failed to create temporary scene audio file");
                    continue;
                }
            };
            if let Err(error) = file.as_file_mut().write_all(package.read_entry(entry)) {
                warn!(%error, resource = %sound.resource, "failed to extract scene audio resource");
                continue;
            }
            prepared.tracks.push(PreparedAudioTrack {
                file,
                resource: sound.resource.clone(),
                loop_playback: sound.playback_mode == SoundPlaybackMode::Loop,
                volume: sound.volume,
            });
        }
        prepared
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn start(&self, enabled: bool) -> SceneAudioRuntime {
        if !enabled {
            if !self.tracks.is_empty() {
                info!(
                    track_count = self.tracks.len(),
                    "scene background music disabled by mute configuration"
                );
            }
            return SceneAudioRuntime {
                playbacks: Vec::new(),
            };
        }
        let playbacks = self
            .tracks
            .iter()
            .filter_map(|track| {
                match AudioPlayback::open_scene(
                    track.file.path(),
                    track.loop_playback,
                    track.volume,
                ) {
                    Ok(playback) => {
                        info!(resource = %track.resource, loop_playback = track.loop_playback, volume = track.volume, "scene background music ready");
                        Some(playback)
                    }
                    Err(error) => {
                        warn!(%error, resource = %track.resource, "scene background music unavailable; continuing without audio");
                        None
                    }
                }
            })
            .collect();
        SceneAudioRuntime { playbacks }
    }
}

fn safe_audio_suffix(resource: &str) -> String {
    let extension = Path::new(resource)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 8
                && extension
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        })
        .unwrap_or("audio");
    format!(".{extension}")
}

#[cfg(test)]
mod tests {
    use better_wallpaper_scene_format::{PkgReader, parse_scene_graph};

    use super::*;

    fn sized_string(value: &str) -> Vec<u8> {
        let mut bytes = (value.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(value.as_bytes());
        bytes
    }

    fn package(entries: &[(&str, &[u8])]) -> PkgReader {
        let mut bytes = sized_string("PKGV0001");
        bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        let mut offset = 0_u32;
        for (name, data) in entries {
            bytes.extend_from_slice(&sized_string(name));
            bytes.extend_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            offset += data.len() as u32;
        }
        for (_, data) in entries {
            bytes.extend_from_slice(data);
        }
        PkgReader::parse(bytes).unwrap()
    }

    #[test]
    fn extracts_one_visible_autostart_background_track() {
        let graph = parse_scene_graph(
            r#"{"objects":[
                {"id":1,"sound":["sounds/music.flac"],"playbackmode":"loop","volume":0.5},
                {"id":2,"sound":"sounds/silent.ogg","startsilent":true},
                {"id":3,"sound":"sounds/hidden.wav","visible":false}
            ]}"#,
        )
        .unwrap();
        let package = package(&[("sounds/music.flac", b"FLAC test bytes")]);

        let prepared = PreparedSceneAudio::prepare(&package, &graph);

        assert_eq!(prepared.track_count(), 1);
        assert_eq!(prepared.start_silent_tracks, 1);
        assert_eq!(prepared.tracks[0].file.path().extension().unwrap(), "flac");
        assert_eq!(
            std::fs::read(prepared.tracks[0].file.path()).unwrap(),
            b"FLAC test bytes"
        );
        assert!(prepared.tracks[0].loop_playback);
        assert_eq!(prepared.tracks[0].volume, 0.5);
    }

    #[test]
    fn missing_sound_is_fail_soft() {
        let graph = parse_scene_graph(r#"{"objects":[{"sound":"missing.ogg"}]}"#).unwrap();
        let prepared = PreparedSceneAudio::prepare(&package(&[]), &graph);
        assert_eq!(prepared.track_count(), 0);
    }
}
