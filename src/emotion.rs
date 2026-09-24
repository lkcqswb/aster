//! What 弄玉 feels and does, decided from her own words and from what actually happened.
//!
//! The model may begin a reply, or a paragraph where the feeling changes, with a cue such as
//! `〔happy〕` or `〔wave〕`. Cues are hidden from the transcript, never sent anywhere else, and
//! only move her face and body. Unknown bracketed text is left exactly as written.

pub const EMOTIONS: &[&str] = &[
    "neutral",
    "happy",
    "laugh",
    "shy",
    "love",
    "excited",
    "surprised",
    "confused",
    "thinking",
    "sad",
    "cry",
    "angry",
    "pout",
    "embarrassed",
    "sleepy",
    "proud",
    "worried",
    "dizzy",
];
pub const GESTURES: &[&str] = &[
    "wave",
    "nod",
    "shake",
    "tilt",
    "think",
    "cheer",
    "heart",
    "cover",
    "stretch",
    "look_around",
    "fidget",
    "bow",
];

#[derive(Clone, Debug, PartialEq)]
pub enum Cue {
    Emotion(String),
    Gesture(String),
}

fn cue(name: &str) -> Option<Cue> {
    let name = name.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    if EMOTIONS.contains(&name.as_str()) {
        Some(Cue::Emotion(name))
    } else if GESTURES.contains(&name.as_str()) {
        Some(Cue::Gesture(name))
    } else {
        None
    }
}

const OPEN: [&str; 2] = ["〔", "[["];
const CLOSE: [&str; 2] = ["〕", "]]"];
/// Longest cue body worth waiting for while streaming.
const LONGEST: usize = 16;

/// Remove every recognized cue from complete text.
pub fn strip(text: &str) -> (String, Vec<Cue>) {
    let mut parser = Cues::default();
    let mut visible = parser.feed(text);
    visible.push_str(&parser.finish());
    let cues = std::mem::take(&mut parser.found);
    (tidy(&visible), cues)
}

/// Remove the blank space a cue leaves at the start of a line.
fn tidy(text: &str) -> String {
    text.lines()
        .map(|l| {
            if l.starts_with(' ') && !l.starts_with("  ") {
                &l[1..]
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if text.ends_with('\n') { "\n" } else { "" }
}

/// Streaming cue parser: text is released as soon as it cannot be part of a cue.
#[derive(Default)]
pub struct Cues {
    held: String,
    pub found: Vec<Cue>,
}
impl Cues {
    pub fn feed(&mut self, text: &str) -> String {
        self.held.push_str(text);
        let mut out = String::new();
        loop {
            let Some((start, open)) = OPEN
                .iter()
                .enumerate()
                .filter_map(|(k, o)| self.held.find(o).map(|i| (i, k)))
                .min()
            else {
                // Keep a trailing "[" that may begin "[[" in the next delta.
                let keep = if self.held.ends_with('[') { 1 } else { 0 };
                out.push_str(&self.held[..self.held.len() - keep]);
                self.held.drain(..self.held.len() - keep);
                return out;
            };
            out.push_str(&self.held[..start]);
            self.held.drain(..start);
            let after = OPEN[open].len();
            match self.held[after..].find(CLOSE[open]) {
                Some(end) => {
                    let body = &self.held[after..after + end];
                    let whole = after + end + CLOSE[open].len();
                    if let Some(c) = cue(body) {
                        self.found.push(c);
                        // Swallow one following space so "〔happy〕 Hello" reads "Hello".
                        let mut skip = whole;
                        if self.held[skip..].starts_with(' ') {
                            skip += 1;
                        }
                        self.held.drain(..skip);
                    } else {
                        out.push_str(&self.held[..after]);
                        self.held.drain(..after);
                    }
                }
                None if self.held[after..].chars().count() > LONGEST
                    || self.held[after..].contains('\n') =>
                {
                    out.push_str(&self.held[..after]);
                    self.held.drain(..after);
                }
                None => return out,
            }
        }
    }
    /// Release anything still held at the end of a reply.
    pub fn finish(&mut self) -> String {
        std::mem::take(&mut self.held)
    }
    pub fn take(&mut self) -> Vec<Cue> {
        std::mem::take(&mut self.found)
    }
}

/// A conservative reading of a finished reply that carried no cue.
pub fn infer(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if has(&["哈哈", "haha", "lol", "😂", "🤣"]) {
        Some("laugh")
    } else if has(&["抱歉", "对不起", "sorry", "apolog", "遗憾", "unfortunately"]) {
        Some("worried")
    } else if has(&["谢谢", "thank you", "thanks", "❤", "💕", "爱你"]) {
        Some("shy")
    } else if has(&[
        "太好了",
        "好耶",
        "成功",
        "passed",
        "great",
        "awesome",
        "🎉",
        "done!",
    ]) {
        Some("happy")
    } else if has(&["哇", "wow", "竟然", "surprising"]) {
        Some("surprised")
    } else if has(&["嗯……", "让我想想", "let me think", "hmm"]) {
        Some("thinking")
    } else {
        None
    }
}

/// Tell the model how to use her face. Kept short: this is sent with every request.
pub fn instructions() -> String {
    format!(
        "Your face and body in the terminal follow cues you write. When it feels natural, begin a reply, or a paragraph where your feeling changes, with one cue in 〔〕 brackets: an emotion ({}) or a gesture ({}). Usually one or two per reply; never explain them. Cues are hidden from Aster. Never put cues inside code, commands, file contents or tool input.",
        EMOTIONS.join(", "),
        GESTURES.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cues_are_hidden_and_other_brackets_survive() {
        let (text, cues) = strip(
            "〔happy〕 写好了！\n\n[[wave]] See [[wiki link]] and 〔not a cue〕 and [x] done.",
        );
        assert_eq!(
            text,
            "写好了！\n\nSee [[wiki link]] and 〔not a cue〕 and [x] done."
        );
        assert_eq!(
            cues,
            [Cue::Emotion("happy".into()), Cue::Gesture("wave".into())]
        );
        assert_eq!(
            strip("〔Look Around〕hi").1,
            [Cue::Gesture("look_around".into())]
        );
    }
    #[test]
    fn streaming_holds_only_what_may_be_a_cue() {
        let mut p = Cues::default();
        let mut shown = String::new();
        for part in ["Hel", "lo 〔ha", "ppy〕 there [", "[nod]] ok [", "x]"] {
            shown.push_str(&p.feed(part));
        }
        shown.push_str(&p.finish());
        assert_eq!(shown, "Hello there ok [x]");
        assert_eq!(
            p.take(),
            [Cue::Emotion("happy".into()), Cue::Gesture("nod".into())]
        );
        // An unclosed bracket is released once it cannot be a cue.
        let mut p = Cues::default();
        let first = p.feed("a 〔this is far too long to be any cue name");
        assert!(first.starts_with("a 〔this"));
        assert_eq!(p.feed("\n"), "\n");
    }
    #[test]
    fn inference_is_conservative() {
        assert_eq!(infer("抱歉，这一步失败了。"), Some("worried"));
        assert_eq!(infer("哈哈，好的"), Some("laugh"));
        assert_eq!(infer("The function returns a list."), None);
        assert!(instructions().contains("〔"));
    }
}
