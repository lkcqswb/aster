//! A bounded, regex-based outline of definitions in one project file. Best effort by design.
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::{Value, json};
use std::path::Path;

const MAX_ENTRIES: usize = 400;
const MAX_OUTPUT: usize = 32_000;
const MAX_NAME: usize = 120;

struct Rule {
    kind: &'static str,
    pattern: Regex,
}
fn rule(kind: &'static str, pattern: &str) -> Rule {
    Rule {
        kind,
        pattern: Regex::new(pattern).expect("outline patterns are valid"),
    }
}

fn language(name: &str, source: &str) -> Option<&'static str> {
    let file = name.rsplit('/').next().unwrap_or(name);
    let extension = file
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    let by_extension = match extension.as_str() {
        "rs" => "rust",
        "py" | "pyi" | "pyw" => "python",
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" => "javascript",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "cs" => "csharp",
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "ino" => "c",
        "rb" | "rake" | "gemspec" => "ruby",
        "sh" | "bash" | "zsh" | "ksh" => "shell",
        "md" | "markdown" | "mdx" => "markdown",
        _ => "",
    };
    if !by_extension.is_empty() {
        return Some(by_extension);
    }
    if matches!(file, "Rakefile" | "Gemfile") {
        return Some("ruby");
    }
    let first = source.lines().next().unwrap_or("");
    if first.starts_with("#!") {
        if first.contains("python") {
            return Some("python");
        }
        if first.contains("ruby") {
            return Some("ruby");
        }
        if first.contains("node") {
            return Some("javascript");
        }
        if ["sh", "bash", "zsh", "ksh", "dash"]
            .iter()
            .any(|s| first.ends_with(&format!("/{s}")) || first.contains(&format!(" {s}")))
        {
            return Some("shell");
        }
    }
    None
}

fn rules(language: &str) -> Vec<Rule> {
    const VIS: &str = r"(?:pub(?:\s*\([^)]*\))?\s+)?";
    match language {
        "rust" => vec![
            rule(
                "fn",
                &format!(
                    r#"^\s*{VIS}(?:default\s+)?(?:(?:const|async|unsafe|extern(?:\s+"[^"]*")?)\s+)*fn\s+(?P<name>[A-Za-z_]\w*)"#
                ),
            ),
            rule(
                "struct",
                &format!(r"^\s*{VIS}struct\s+(?P<name>[A-Za-z_]\w*)"),
            ),
            rule("enum", &format!(r"^\s*{VIS}enum\s+(?P<name>[A-Za-z_]\w*)")),
            rule(
                "union",
                &format!(r"^\s*{VIS}union\s+(?P<name>[A-Za-z_]\w*)"),
            ),
            rule(
                "trait",
                &format!(r"^\s*{VIS}(?:unsafe\s+)?(?:auto\s+)?trait\s+(?P<name>[A-Za-z_]\w*)"),
            ),
            rule("impl", r"^\s*(?:unsafe\s+)?impl\b(?P<name>.*)"),
            rule("mod", &format!(r"^\s*{VIS}mod\s+(?P<name>[A-Za-z_]\w*)")),
            rule(
                "const",
                &format!(r"^\s*{VIS}(?:const|static(?:\s+mut)?)\s+(?P<name>[A-Za-z_]\w*)\s*:"),
            ),
            rule("type", &format!(r"^\s*{VIS}type\s+(?P<name>[A-Za-z_]\w*)")),
            rule("macro", r"^\s*macro_rules!\s*(?P<name>[A-Za-z_]\w*)"),
        ],
        "python" => vec![
            rule("def", r"^\s*(?:async\s+)?def\s+(?P<name>[A-Za-z_]\w*)"),
            rule("class", r"^\s*class\s+(?P<name>[A-Za-z_]\w*)"),
        ],
        "javascript" => {
            const EXPORT: &str = r"(?:export\s+)?(?:default\s+)?(?:declare\s+)?";
            vec![
                rule(
                    "function",
                    &format!(r"^\s*{EXPORT}(?:async\s+)?function\s*\*?\s*(?P<name>[\w$]+)"),
                ),
                rule(
                    "class",
                    &format!(r"^\s*{EXPORT}(?:abstract\s+)?class\s+(?P<name>[\w$]+)"),
                ),
                rule(
                    "interface",
                    &format!(r"^\s*{EXPORT}interface\s+(?P<name>[\w$]+)"),
                ),
                rule(
                    "type",
                    &format!(r"^\s*{EXPORT}type\s+(?P<name>[\w$]+)\s*(?:<.*>)?\s*="),
                ),
                rule(
                    "enum",
                    &format!(r"^\s*{EXPORT}(?:const\s+)?enum\s+(?P<name>[\w$]+)"),
                ),
                rule(
                    "const",
                    r"^\s*(?:export\s+)?(?:const|let|var)\s+(?P<name>[\w$]+)\s*(?::[^=]+)?=\s*(?:async\s+)?(?:function\b|\([^)]*\)\s*(?::[^=]+)?=>|[\w$]+\s*=>)",
                ),
                rule(
                    "export",
                    r"^\s*export\s+(?:default\s+)?(?:const|let|var)\s+(?P<name>[\w$]+)",
                ),
                rule(
                    "method",
                    r"^\s+(?:(?:public|private|protected|static|readonly|override|abstract|async|get|set)\s+)*\*?(?P<name>[A-Za-z_$][\w$]*)\s*(?:<[^>]*>)?\s*\([^;]*\)\s*(?::\s*[^;{]+)?\{\s*$",
                ),
            ]
        }
        "go" => vec![
            rule(
                "func",
                r"^func\s+(?:\((?P<receiver>[^)]*)\)\s*)?(?P<name>[A-Za-z_]\w*)",
            ),
            rule("type", r"^\s*type\s+(?P<name>[A-Za-z_]\w*)\s+\S"),
        ],
        "java" | "kotlin" | "csharp" => {
            const MODIFIERS: &str = r"(?:(?:public|private|protected|internal|static|final|abstract|sealed|partial|open|data|inner|readonly|ref|unsafe|new|override|virtual|async|synchronized|native|extern|default|strictfp|transient|volatile|suspend|inline|operator|infix|tailrec|external|lateinit|annotation|value|enum|companion|const)\s+)";
            vec![
                rule(
                    "class",
                    &format!(
                        r"^\s*(?:@\w+(?:\([^)]*\))?\s+)*{MODIFIERS}*(?P<kind>class|interface|enum|record|struct|object|namespace)\s+(?P<name>[A-Za-z_]\w*)"
                    ),
                ),
                rule(
                    "fun",
                    &format!(
                        r"^\s*{MODIFIERS}*fun\s+(?:<[^>]*>\s*)?(?:[\w.]+\.)?(?P<name>[A-Za-z_]\w*)"
                    ),
                ),
                rule(
                    "method",
                    &format!(
                        r"^\s*{MODIFIERS}+(?:<[^>]*>\s*)?[\w<>\[\],.?]+(?:\s*<[^>]*>)?\s+(?P<name>[A-Za-z_]\w*)\s*\([^;]*$"
                    ),
                ),
            ]
        }
        "c" => vec![
            rule(
                "type",
                r"^\s*(?:typedef\s+)?(?:template\s*<.*>\s*)?(?P<kind>class|struct|union|enum(?:\s+class)?|namespace)\s+(?:\w+\s+)*?(?P<name>[A-Za-z_]\w*)\s*(?:final\s*)?(?:[:{]|$)",
            ),
            rule(
                "function",
                r"^(?:(?:static|inline|extern|virtual|constexpr|explicit|friend|const|unsigned|signed|struct|enum)\s+)*[A-Za-z_][\w:<>,]*(?:\s*[*&]+\s*|\s+)(?P<name>~?[A-Za-z_][\w:~]*)\s*\([^;]*\)\s*(?:const\s*)?(?:noexcept\s*)?(?:override\s*)?(?:\{.*)?$",
            ),
        ],
        "ruby" => vec![
            rule(
                "def",
                r"^\s*def\s+(?P<name>(?:self\.)?[A-Za-z_][\w]*[?!=]?)",
            ),
            rule("class", r"^\s*class\s+(?P<name>[A-Z][\w:]*)"),
            rule("module", r"^\s*module\s+(?P<name>[A-Z][\w:]*)"),
        ],
        "shell" => vec![
            rule(
                "function",
                r"^\s*function\s+(?P<name>[A-Za-z_][\w.:-]*)\s*(?:\(\s*\))?\s*(?:\{.*)?$",
            ),
            rule(
                "function",
                r"^\s*(?P<name>[A-Za-z_][\w.:-]*)\s*\(\s*\)\s*(?:\{.*)?$",
            ),
        ],
        _ => vec![],
    }
}

const NOT_NAMES: &[&str] = &[
    "if", "for", "while", "switch", "catch", "return", "else", "do", "sizeof", "new", "delete",
    "throw", "function", "typeof", "await", "yield", "case",
];

/// Clean `impl<T: Into<String>> Trait for Type where …` into `Trait for Type`.
fn impl_name(text: &str) -> String {
    let mut rest = text.trim_start();
    if rest.starts_with('<') {
        let mut depth = 0;
        for (i, c) in rest.char_indices() {
            match c {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        rest = &rest[i + 1..];
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    let rest = rest.split('{').next().unwrap_or(rest);
    let rest = rest.split(" where").next().unwrap_or(rest);
    rest.trim().to_string()
}

fn markdown(source: &str, entries: &mut Vec<Value>, found: &mut usize, bytes: &mut usize) {
    let heading = Regex::new(r"^ {0,3}(?P<level>#{1,6})\s+(?P<name>.*?)(?:\s+#+)?\s*$")
        .expect("heading pattern is valid");
    let mut fence: Option<String> = None;
    let mut previous = "";
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(open) = &fence {
            if trimmed.starts_with(open.as_str()) {
                fence = None;
            }
            previous = "";
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(trimmed[..3].to_string());
            previous = "";
            continue;
        }
        if let Some(c) = heading.captures(line) {
            let level = c["level"].len();
            push(
                entries,
                found,
                bytes,
                index + 1,
                &format!("h{level}"),
                &c["name"],
                0,
            );
        } else if !previous.trim().is_empty()
            && !previous.trim_start().starts_with(['#', '-', '*', '>', '|'])
            && (line.trim_end().chars().all(|c| c == '=')
                || line.trim_end().chars().all(|c| c == '-'))
            && line.trim_end().len() >= 2
            && !line.starts_with(' ')
        {
            let kind = if line.starts_with('=') { "h1" } else { "h2" };
            push(entries, found, bytes, index, kind, previous.trim(), 0);
        }
        previous = line;
    }
}

fn push(
    entries: &mut Vec<Value>,
    found: &mut usize,
    bytes: &mut usize,
    line: usize,
    kind: &str,
    name: &str,
    indent: usize,
) {
    *found += 1;
    let entry =
        json!({"line":line,"kind":kind,"name":crate::tools::clip(name, MAX_NAME),"indent":indent});
    let size = entry.to_string().len() + 1;
    if entries.len() < MAX_ENTRIES && *bytes + size <= MAX_OUTPUT {
        *bytes += size;
        entries.push(entry);
    }
}

pub fn outline(root: &Path, args: &Value) -> Result<Value> {
    let name = args["path"].as_str().context("path must be text")?;
    let source = crate::project::text(root, name)?;
    let Some(language) = language(name, &source) else {
        bail!(
            "outline supports Rust, Python, JavaScript/TypeScript, Go, Java, Kotlin, C#, C/C++, Ruby, shell and Markdown files; use search or read_file for this file"
        );
    };
    let mut entries = Vec::new();
    let (mut found, mut bytes) = (0, 0);
    if language == "markdown" {
        markdown(&source, &mut entries, &mut found, &mut bytes);
    } else {
        let rules = rules(language);
        let mut block_comment = false;
        for (index, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            if language != "python" && language != "ruby" && language != "shell" {
                if block_comment {
                    block_comment = !line.contains("*/");
                    continue;
                }
                if trimmed.starts_with("/*") && !trimmed.contains("*/") {
                    block_comment = true;
                    continue;
                }
                if trimmed.starts_with("//") || trimmed.starts_with('*') {
                    continue;
                }
            } else if trimmed.starts_with('#') {
                continue;
            }
            for rule in &rules {
                let Some(captures) = rule.pattern.captures(line) else {
                    continue;
                };
                let Some(found_name) = captures.name("name") else {
                    continue;
                };
                let mut label = found_name.as_str().to_string();
                if NOT_NAMES.contains(&label.as_str()) {
                    continue;
                }
                let mut kind = captures
                    .name("kind")
                    .map_or(rule.kind, |k| match k.as_str() {
                        "class" => "class",
                        "interface" => "interface",
                        "enum" => "enum",
                        "record" => "record",
                        "struct" => "struct",
                        "object" => "object",
                        "union" => "union",
                        "namespace" => "namespace",
                        _ => "enum",
                    });
                if rule.kind == "impl" {
                    label = impl_name(&label);
                    if label.is_empty() {
                        continue;
                    }
                }
                if rule.kind == "func" && captures.name("receiver").is_some() {
                    kind = "method";
                }
                let indent = line.len() - trimmed.len();
                push(
                    &mut entries,
                    &mut found,
                    &mut bytes,
                    index + 1,
                    kind,
                    &label,
                    indent,
                );
                break;
            }
        }
    }
    let truncated = entries.len() < found;
    Ok(json!({
        "path": name,
        "language": language,
        "entries": entries,
        "found": found,
        "truncated": truncated,
        "note": if truncated {
            "Outline limit reached (400 entries or 32 KB). Use search or read_file for the rest."
        } else {
            "Regex-based outline; read the lines before relying on a definition."
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn outline_of(name: &str, source: &str) -> Vec<(u64, String, String)> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, source).unwrap();
        let result = outline(dir.path(), &json!({"path":name})).unwrap();
        result["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["line"].as_u64().unwrap(),
                    e["kind"].as_str().unwrap().to_string(),
                    e["name"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }
    fn names(entries: &[(u64, String, String)]) -> Vec<String> {
        entries
            .iter()
            .map(|(line, kind, name)| format!("{line} {kind} {name}"))
            .collect()
    }

    #[test]
    fn rust_definitions_have_one_based_lines() {
        let source = "//! doc\nuse std::fmt;\npub struct Point { x: i32 }\nenum Shape {}\npub(crate) trait Draw {}\nimpl<T: Into<String>> Draw for Wrapper<T> where T: Clone {\n    pub async fn draw(&self) {}\n}\nmod inner {}\npub const LIMIT: usize = 3;\nmacro_rules! square { () => {} }\npub unsafe extern \"C\" fn raw() {}\n// fn commented() {}\nconst fn fixed() -> u8 { 1 }\ntype Alias = u8;\n";
        assert_eq!(
            names(&outline_of("src/lib.rs", source)),
            [
                "3 struct Point",
                "4 enum Shape",
                "5 trait Draw",
                "6 impl Draw for Wrapper<T>",
                "7 fn draw",
                "9 mod inner",
                "10 const LIMIT",
                "11 macro square",
                "12 fn raw",
                "14 fn fixed",
                "15 type Alias",
            ]
        );
    }

    #[test]
    fn python_javascript_go_ruby_shell_and_markdown_are_outlined() {
        assert_eq!(
            names(&outline_of(
                "app.py",
                "import os\nclass Pet:\n    def speak(self):\n        pass\nasync def main():\n    # def hidden():\n    pass\n"
            )),
            ["2 class Pet", "3 def speak", "5 def main"]
        );
        assert_eq!(
            names(&outline_of(
                "web/app.ts",
                "export function start() {}\nexport default class App {\n  render(): void {\n    if (ready) {\n    }\n  }\n}\ninterface Props {}\nexport type Id = string;\nconst add = (a, b) => a + b;\nexport const VERSION = '1';\nenum Mode { A }\n"
            )),
            [
                "1 function start",
                "2 class App",
                "3 method render",
                "8 interface Props",
                "9 type Id",
                "10 const add",
                "11 export VERSION",
                "12 enum Mode",
            ]
        );
        assert_eq!(
            names(&outline_of(
                "main.go",
                "package main\ntype Server struct {}\nfunc (s *Server) Run() error { return nil }\nfunc main() {}\n"
            )),
            ["2 type Server", "3 method Run", "4 func main"]
        );
        assert_eq!(
            names(&outline_of(
                "lib/pet.rb",
                "module Zoo\n  class Pet\n    def self.create\n    end\n    def ready?\n    end\n  end\nend\n"
            )),
            [
                "1 module Zoo",
                "2 class Pet",
                "3 def self.create",
                "5 def ready?"
            ]
        );
        assert_eq!(
            names(&outline_of(
                "run.sh",
                "#!/bin/sh\nsetup() {\n  :\n}\nfunction cleanup {\n  :\n}\n"
            )),
            ["2 function setup", "5 function cleanup"]
        );
        assert_eq!(
            names(&outline_of(
                "README.md",
                "# Title\n\nText\n\n```sh\n# not a heading\n```\n## Usage ##\nSetext\n------\n"
            )),
            ["1 h1 Title", "8 h2 Usage", "9 h2 Setext"]
        );
    }

    #[test]
    fn c_family_outlines_are_best_effort() {
        assert_eq!(
            names(&outline_of(
                "Main.java",
                "package app;\n@Service\npublic final class Main {\n    private static int count(List<String> items) {\n        return items.size();\n    }\n    public interface Listener {}\n}\n"
            )),
            ["3 class Main", "4 method count", "7 interface Listener"]
        );
        assert_eq!(
            names(&outline_of(
                "Pet.kt",
                "data class Pet(val name: String)\nsuspend fun load(): Pet = TODO()\nobject Registry\n"
            )),
            ["1 class Pet", "2 fun load", "3 object Registry"]
        );
        assert_eq!(
            names(&outline_of(
                "src/pet.cpp",
                "#include <x>\nnamespace zoo {\nstruct Pet {\n};\nstatic int count(const char *name) {\n  return 0;\n}\nvoid Pet::speak() const {\n}\n}\n"
            )),
            [
                "2 namespace zoo",
                "3 struct Pet",
                "5 function count",
                "8 function Pet::speak",
            ]
        );
        assert_eq!(
            names(&outline_of(
                "Program.cs",
                "namespace App {\n  public sealed record Pet(string Name);\n  internal class Store {\n    public async Task<int> CountAsync(int x) {\n    }\n  }\n}\n"
            )),
            [
                "1 namespace App",
                "2 record Pet",
                "3 class Store",
                "4 method CountAsync",
            ]
        );
    }

    #[test]
    fn outline_is_bounded_and_rejects_unknown_or_private_files() {
        let dir = tempfile::tempdir().unwrap();
        let source = (0..1000)
            .map(|i| format!("fn f{i}() {{}}\n"))
            .collect::<String>();
        fs::write(dir.path().join("many.rs"), source).unwrap();
        let result = outline(dir.path(), &json!({"path":"many.rs"})).unwrap();
        assert_eq!(result["entries"].as_array().unwrap().len(), MAX_ENTRIES);
        assert_eq!(result["found"], 1000);
        assert_eq!(result["truncated"], true);
        assert!(result.to_string().len() < 40_000);
        fs::write(dir.path().join("data.bin"), "abc").unwrap();
        assert!(outline(dir.path(), &json!({"path":"data.bin"})).is_err());
        fs::write(dir.path().join(".env"), "fn secret() {}").unwrap();
        assert!(outline(dir.path(), &json!({"path":".env"})).is_err());
        assert!(outline(dir.path(), &json!({"path":"../x.rs"})).is_err());
    }
}
