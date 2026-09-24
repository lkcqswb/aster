//! Public, declarative companion profiles. No executable code or model assets.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Read, path::Path};

pub const DEFAULT_PROFILE: &str = include_str!("../live2d/profiles/nongyu.json");
pub const STATES: &[&str] = &[
    "idle",
    "listening",
    "thinking",
    "reading",
    "working",
    "checking",
    "waiting",
    "speaking",
    "pleased",
    "concerned",
];
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub zoom: f64,
    pub center: [f64; 2],
    pub anchor: [f64; 2],
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub duration_ms: u32,
    pub blend: String,
    pub tracks: BTreeMap<String, Vec<[f64; 2]>>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub version: u32,
    pub name: String,
    pub model: String,
    pub layout: Layout,
    pub bindings: BTreeMap<String, String>,
    pub emotions: BTreeMap<String, BTreeMap<String, f64>>,
    pub motions: BTreeMap<String, Clip>,
}
pub fn relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && !path.starts_with('/')
        && !path.contains(['\\', ':', '?', '#', '%'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}
fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}
impl Profile {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let text = if let Some(path) = path {
            let mut text = String::new();
            fs::File::open(path)?
                .take(128_001)
                .read_to_string(&mut text)?;
            if text.len() > 128_000 {
                bail!("Companion profile exceeds 128 KB")
            }
            text
        } else {
            DEFAULT_PROFILE.into()
        };
        let profile: Self =
            serde_json::from_str(&text).context("Invalid companion profile JSON")?;
        profile.validate()?;
        Ok(profile)
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.name.is_empty()
            || self.name.len() > 120
            || self.name.chars().any(char::is_control)
            || !relative(&self.model)
            || !self.model.ends_with(".model3.json")
        {
            bail!(
                "Use companion version 1, a name up to 120 bytes, and a relative .model3.json path"
            );
        }
        if !self.layout.zoom.is_finite()
            || !(0.1..=5.0).contains(&self.layout.zoom)
            || self
                .layout
                .center
                .iter()
                .chain(self.layout.anchor.iter())
                .any(|n| !n.is_finite() || !(0.0..=1.0).contains(n))
        {
            bail!("Layout zoom must be 0.1–5; center and anchor coordinates must be 0–1");
        }
        if self.bindings.is_empty()
            || self.bindings.len() > 128
            || self.bindings.iter().any(|(k, v)| !name(k) || !name(v))
            || self
                .bindings
                .values()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.bindings.len()
        {
            bail!("Use 1–128 unique parameter bindings with ASCII names up to 64 bytes");
        }
        if !self.emotions.contains_key("neutral")
            || self.emotions.len() > 32
            || self.motions.len() > 32
        {
            bail!("Define neutral and at most 32 emotions and 32 motions");
        }
        let valid_value = |v: &f64| v.is_finite() && v.abs() <= 10_000.0;
        for (emotion, values) in &self.emotions {
            if !name(emotion)
                || values
                    .iter()
                    .any(|(key, value)| !self.bindings.contains_key(key) || !valid_value(value))
            {
                bail!(
                    "Invalid emotion {emotion}: use bound channels and finite values within ±10000"
                );
            }
        }
        for (motion, clip) in &self.motions {
            if !name(motion)
                || !(100..=10_000).contains(&clip.duration_ms)
                || !matches!(clip.blend.as_str(), "add" | "set")
                || clip.tracks.is_empty()
                || clip.tracks.len() > 128
            {
                bail!(
                    "Invalid motion {motion}: use 100–10000 ms, add/set blending and 1–128 tracks"
                );
            }
            for (channel, frames) in &clip.tracks {
                if !self.bindings.contains_key(channel)
                    || !(2..=64).contains(&frames.len())
                    || frames[0][0] != 0.0
                    || frames.last().unwrap()[0] != 1.0
                    || frames.iter().any(|[at, v]| {
                        !at.is_finite() || !(0.0..=1.0).contains(at) || !valid_value(v)
                    })
                    || frames.windows(2).any(|pair| pair[0][0] >= pair[1][0])
                {
                    bail!(
                        "Invalid track {motion}/{channel}: use 2–64 ordered [time,value] frames from time 0 to 1"
                    );
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Control {
    Emotion { name: String, strength: f64 },
    Motion { name: String, strength: f64 },
    Look { x: f64, y: f64, duration_ms: u32 },
    Reset,
}
impl Control {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Emotion { name: n, strength } | Self::Motion { name: n, strength } => {
                if !name(n) || !strength.is_finite() || !(0.0..=1.0).contains(strength) {
                    bail!("Use a profile name and strength from 0 to 1")
                }
            }
            Self::Look { x, y, duration_ms } => {
                if !x.is_finite()
                    || !y.is_finite()
                    || x.abs() > 1.0
                    || y.abs() > 1.0
                    || !(100..=10000).contains(duration_ms)
                {
                    bail!("Gaze coordinates must be −1 to 1, duration 100–10000 ms")
                }
            }
            Self::Reset => {}
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_and_controls_reject_unsafe_or_unbounded_input() {
        let mut p = Profile::load(None).unwrap();
        assert_eq!(p.version, 1);
        p.model = "../private.model3.json".into();
        assert!(p.validate().is_err());
        p = Profile::load(None).unwrap();
        p.motions
            .get_mut("nod")
            .unwrap()
            .tracks
            .get_mut("head_y")
            .unwrap()[1][0] = 0.0;
        assert!(p.validate().is_err());
        assert!(
            Control::Emotion {
                name: "happy".into(),
                strength: f64::NAN
            }
            .validate()
            .is_err()
        );
        assert!(
            Control::Look {
                x: 2.0,
                y: 0.0,
                duration_ms: 1000
            }
            .validate()
            .is_err()
        );
        assert!(!relative("https://example.com/rig.model3.json"));
        assert!(!relative("model/%2e%2e/file"));
        assert!(relative("my pet/rig.model3.json"));
        let extra = DEFAULT_PROFILE.replacen(
            "\"version\": 1",
            "\"version\": 1, \"javascript\": \"execute()\"",
            1,
        );
        assert!(serde_json::from_str::<Profile>(&extra).is_err());
    }
}
