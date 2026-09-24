//! Markdown as inert styled terminal text; never emits HTML or terminal escape sequences.
use crate::{theme::*, ui::clean};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

fn style(color: Color) -> Style {
    Style::default().fg(color).bg(BG)
}
fn span(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(clean(&text.into()), style)
}
#[derive(Clone)]
struct Glyph {
    text: String,
    style: Style,
    width: usize,
}
fn merge(glyphs: &[Glyph]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![];
    for g in glyphs {
        if let Some(last) = spans.last_mut()
            && last.style == g.style
        {
            last.content.to_mut().push_str(&g.text);
        } else {
            spans.push(span(&g.text, g.style));
        }
    }
    Line::from(spans)
}
/// Preserve styles and grapheme clusters while wrapping. Code keeps whitespace exactly.
pub fn wrap(spans: &[Span<'_>], width: usize, code: bool) -> Vec<Line<'static>> {
    let width = width.max(1);
    let glyphs = spans
        .iter()
        .flat_map(|s| {
            s.content.graphemes(true).map(|g| Glyph {
                text: g.into(),
                style: s.style,
                width: g.width(),
            })
        })
        .collect::<Vec<_>>();
    let mut result = vec![];
    let mut start = 0;
    while start < glyphs.len() {
        let mut end = start;
        let mut used = 0;
        let mut space = None;
        while end < glyphs.len()
            && glyphs[end].text != "\n"
            && (used + glyphs[end].width <= width || end == start)
        {
            used += glyphs[end].width;
            if glyphs[end].text.chars().all(char::is_whitespace) {
                space = Some(end);
            }
            end += 1;
        }
        let newline = end < glyphs.len() && glyphs[end].text == "\n";
        let mut cut = end;
        let mut next = if newline { end + 1 } else { end };
        if !code
            && !newline
            && end < glyphs.len()
            && let Some(at) = space
            && at > start
        {
            cut = at;
            next = at + 1;
        }
        if !code {
            while cut > start && glyphs[cut - 1].text.chars().all(char::is_whitespace) {
                cut -= 1;
            }
            while next < glyphs.len()
                && glyphs[next].text != "\n"
                && glyphs[next].text.chars().all(char::is_whitespace)
            {
                next += 1;
            }
        }
        result.push(merge(&glyphs[start..cut]));
        start = next;
    }
    if result.is_empty() {
        result.push(Line::default());
    }
    result
}
#[derive(Default)]
struct List {
    next: Option<u64>,
    marker: String,
    first: bool,
}
#[derive(Default)]
struct Table {
    rows: Vec<Vec<Vec<Span<'static>>>>,
    row: Vec<Vec<Span<'static>>>,
    align: Vec<Alignment>,
}
struct Render {
    width: usize,
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    styles: Vec<Style>,
    lists: Vec<List>,
    quotes: usize,
    code: Option<(String, String)>,
    links: Vec<String>,
    table: Option<Table>,
}
impl Render {
    fn current_style(&self) -> Style {
        self.styles.last().copied().unwrap_or_else(|| style(FG))
    }
    fn text(&mut self, text: &str) {
        self.current.push(span(text, self.current_style()));
    }
    fn blank(&mut self) {
        if self.lines.last().is_some_and(|l| !l.spans.is_empty()) {
            self.lines.push(Line::default());
        }
    }
    fn prefix(&self, first: bool) -> String {
        let mut prefix = "│ ".repeat(self.quotes);
        for (i, list) in self.lists.iter().enumerate() {
            if first && i + 1 == self.lists.len() && list.first {
                prefix.push_str(&list.marker);
            } else {
                prefix.push_str(&" ".repeat(list.marker.width()));
            }
        }
        prefix.chars().take(self.width.saturating_sub(4)).collect()
    }
    fn emit(&mut self, rows: Vec<Line<'static>>) {
        for (i, mut row) in rows.into_iter().enumerate() {
            let prefix = self.prefix(i == 0);
            if !prefix.is_empty() {
                row.spans.insert(0, span(prefix, style(DIM)));
            }
            self.lines.push(row);
        }
        if let Some(list) = self.lists.last_mut() {
            list.first = false;
        }
    }
    fn flush(&mut self) {
        if self.current.is_empty() {
            return;
        }
        let width = self.width.saturating_sub(self.prefix(true).width()).max(1);
        let rows = wrap(&self.current, width, false);
        self.current.clear();
        self.emit(rows);
    }
    fn push_style(&mut self, extra: Style) {
        self.styles.push(self.current_style().patch(extra));
    }
    fn pop_style(&mut self) {
        self.styles.pop();
    }
    fn code_block(&mut self, language: String, text: String) {
        let is_diff = matches!(language.as_str(), "diff" | "patch");
        let available = self.width.saturating_sub(self.prefix(false).width());
        if !language.is_empty() {
            self.emit(wrap(&[span(language, style(DIM))], available, true));
        }
        let bar = if available >= 3 { "│ " } else { "" };
        let width = available.saturating_sub(bar.width()).max(1);
        let mut rows = vec![];
        for line in text.lines() {
            let rendered = if is_diff {
                diff(line, width)
            } else {
                wrap(&[span(line, style(FG))], width, true)
            };
            for mut row in rendered {
                row.spans.insert(0, span(bar, style(DIM)));
                rows.push(row);
            }
        }
        self.emit(rows);
        self.blank();
    }
    fn render_table(&mut self, table: Table) {
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return;
        }
        let available = self.width.saturating_sub(self.prefix(false).width());
        let mut widths = (0..columns)
            .map(|col| {
                table
                    .rows
                    .iter()
                    .filter_map(|row| row.get(col))
                    .map(|cell| cell.iter().map(|s| s.content.width()).sum::<usize>())
                    .max()
                    .unwrap_or(0)
                    .clamp(3, 40)
            })
            .collect::<Vec<_>>();
        if available < columns * 4 - 3 {
            // A narrow terminal gets complete stacked cells rather than clipped columns.
            for (index, row) in table.rows.iter().enumerate() {
                self.emit(wrap(
                    &[span(
                        if index == 0 {
                            "Columns".into()
                        } else {
                            format!("Row {index}")
                        },
                        style(JADE),
                    )],
                    available.max(1),
                    false,
                ));
                for (col, cell) in row.iter().enumerate() {
                    let mut value = vec![span(format!("{}. ", col + 1), style(DIM))];
                    value.extend(cell.clone());
                    self.emit(wrap(&value, available.max(1), false));
                }
                self.blank();
            }
            return;
        }
        while widths.iter().sum::<usize>() + (columns - 1) * 3 > available {
            let at = widths
                .iter()
                .enumerate()
                .max_by_key(|(_, w)| **w)
                .map(|(i, _)| i)
                .unwrap();
            widths[at] -= 1;
        }
        for (row_index, row) in table.rows.iter().enumerate() {
            let cells = (0..columns)
                .map(|col| {
                    wrap(
                        row.get(col).map(Vec::as_slice).unwrap_or(&[]),
                        widths[col],
                        false,
                    )
                })
                .collect::<Vec<_>>();
            for height in 0..cells.iter().map(Vec::len).max().unwrap_or(1) {
                let mut rendered = vec![];
                for col in 0..columns {
                    if col > 0 {
                        rendered.push(span(" │ ", style(LINE)));
                    }
                    let cell = cells[col].get(height).cloned().unwrap_or_default();
                    let pad = widths[col].saturating_sub(cell.width());
                    let left = match table.align.get(col) {
                        Some(Alignment::Right) => pad,
                        Some(Alignment::Center) => pad / 2,
                        _ => 0,
                    };
                    rendered.push(span(" ".repeat(left), style(FG)));
                    rendered.extend(cell.spans.into_iter().map(|mut s| {
                        if row_index == 0 {
                            s.style = s.style.add_modifier(Modifier::BOLD);
                        }
                        s
                    }));
                    rendered.push(span(" ".repeat(pad - left), style(FG)));
                }
                self.emit(vec![Line::from(rendered)]);
            }
            if row_index == 0 {
                self.emit(vec![Line::from(span(
                    widths
                        .iter()
                        .map(|w| "─".repeat(*w))
                        .collect::<Vec<_>>()
                        .join("─┼─"),
                    style(LINE),
                ))]);
            }
        }
        self.blank();
    }
}

pub fn markdown(text: &str, width: usize) -> Vec<Line<'static>> {
    let text = clean(text);
    let mut r = Render {
        width: width.max(1),
        lines: vec![],
        current: vec![],
        styles: vec![],
        lists: vec![],
        quotes: 0,
        code: None,
        links: vec![],
        table: None,
    };
    for event in Parser::new_ext(
        &text,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES,
    ) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => r.flush(),
                Tag::Heading { .. } => {
                    r.flush();
                    r.blank();
                    r.push_style(style(JADE).add_modifier(Modifier::BOLD));
                }
                Tag::Emphasis => r.push_style(Style::default().add_modifier(Modifier::ITALIC)),
                Tag::Strong => r.push_style(Style::default().add_modifier(Modifier::BOLD)),
                Tag::Strikethrough => {
                    r.push_style(Style::default().add_modifier(Modifier::CROSSED_OUT))
                }
                Tag::BlockQuote(_) => {
                    r.flush();
                    r.quotes += 1;
                }
                Tag::List(next) => {
                    r.flush();
                    r.lists.push(List {
                        next,
                        ..Default::default()
                    });
                }
                Tag::Item => {
                    r.flush();
                    if let Some(list) = r.lists.last_mut() {
                        list.marker = if let Some(n) = &mut list.next {
                            let marker = format!("{n}. ");
                            *n = n.saturating_add(1);
                            marker
                        } else {
                            "• ".into()
                        };
                        list.first = true;
                    }
                }
                Tag::CodeBlock(kind) => {
                    r.flush();
                    r.blank();
                    r.code = Some((
                        match kind {
                            CodeBlockKind::Fenced(s) => {
                                s.split_whitespace().next().unwrap_or("").into()
                            }
                            _ => String::new(),
                        },
                        String::new(),
                    ));
                }
                Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                    r.links.push(dest_url.into_string());
                    r.push_style(Style::default().fg(JADE).add_modifier(Modifier::UNDERLINED));
                }
                Tag::Table(align) => {
                    r.flush();
                    r.blank();
                    r.table = Some(Table {
                        align,
                        ..Default::default()
                    });
                }
                Tag::TableCell => {
                    r.current.clear();
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    r.flush();
                    if r.lists.is_empty() {
                        r.blank();
                    }
                }
                TagEnd::Heading(_) => {
                    r.flush();
                    r.pop_style();
                    r.blank();
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => r.pop_style(),
                TagEnd::BlockQuote(_) => {
                    r.flush();
                    r.quotes = r.quotes.saturating_sub(1);
                    r.blank();
                }
                TagEnd::Item => r.flush(),
                TagEnd::List(_) => {
                    r.flush();
                    r.lists.pop();
                    if r.lists.is_empty() {
                        r.blank();
                    }
                }
                TagEnd::CodeBlock => {
                    if let Some((language, text)) = r.code.take() {
                        r.code_block(language, text);
                    }
                }
                TagEnd::Link | TagEnd::Image => {
                    r.pop_style();
                    if let Some(url) = r.links.pop() {
                        r.current.push(span(format!(" ({url})"), style(DIM)));
                    }
                }
                TagEnd::TableCell => {
                    if let Some(table) = &mut r.table {
                        table.row.push(std::mem::take(&mut r.current));
                    }
                }
                TagEnd::TableHead | TagEnd::TableRow => {
                    if let Some(table) = &mut r.table {
                        table.rows.push(std::mem::take(&mut table.row));
                    }
                }
                TagEnd::Table => {
                    if let Some(table) = r.table.take() {
                        r.render_table(table);
                    }
                }
                TagEnd::HtmlBlock => {
                    r.flush();
                    r.blank();
                }
                _ => {}
            },
            Event::Text(text) => {
                if let Some((_, code)) = &mut r.code {
                    code.push_str(&text);
                } else {
                    r.text(&text);
                }
            }
            Event::Code(text) => r
                .current
                .push(span(text.into_string(), r.current_style().fg(GOLD))),
            Event::SoftBreak => r.text(" "),
            Event::HardBreak => r.text("\n"),
            Event::Rule => {
                r.flush();
                r.emit(vec![Line::from(span(
                    "─".repeat(r.width.saturating_sub(r.prefix(true).width()).min(36)),
                    style(LINE),
                ))]);
                r.blank();
            }
            Event::TaskListMarker(done) => r
                .current
                .push(span(if done { "☑ " } else { "☐ " }, style(JADE))),
            Event::Html(text)
            | Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
            | Event::FootnoteReference(text) => r.text(&text),
        }
    }
    r.flush();
    while r.lines.last().is_some_and(|l| l.spans.is_empty()) {
        r.lines.pop();
    }
    r.lines
}

pub fn diff(text: &str, width: usize) -> Vec<Line<'static>> {
    clean(text)
        .lines()
        .flat_map(|text| {
            let color = if text.starts_with("+++") || text.starts_with("---") {
                DIM
            } else if text.starts_with('+') {
                JADE
            } else if text.starts_with('-') {
                RED
            } else if text.starts_with("@@") {
                GOLD
            } else {
                FG
            };
            wrap(&[span(text, style(color))], width, true)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plain(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn markdown_styles_have_real_text_without_markup_and_preserve_link_targets() {
        let lines = markdown(
            "# A clear heading\n\n**Bold** and *emphasis* with `x = 1`.\n\n- [x] Done\n- [ ] Pending\n\n[Source](https://example.com/page)",
            60,
        );
        let text = plain(&lines);
        assert!(
            text.contains("A clear heading")
                && text.contains("☑ Done")
                && text.contains("☐ Pending")
        );
        assert!(text.contains("Source (https://example.com/page)"));
        assert!(!text.contains("**") && !text.contains("`"));
        assert!(
            lines
                .iter()
                .flat_map(|l| &l.spans)
                .any(|s| s.content == "Bold" && s.style.add_modifier.contains(Modifier::BOLD))
        );
        assert!(
            lines
                .iter()
                .flat_map(|l| &l.spans)
                .any(|s| s.content == "x = 1" && s.style.fg == Some(GOLD))
        );
        assert!(lines.iter().all(|l| l.width() <= 60));
    }
    #[test]
    fn code_preserves_indentation_literal_markdown_and_diff_colors() {
        let lines = markdown(
            "```rust\nfn demo() {\n    let literal = \"**keep_this**\";  \n}\n```\n\n```diff\n-old\n+new\n```",
            70,
        );
        let text = plain(&lines);
        assert!(text.contains("│     let literal = \"**keep_this**\";  \n"));
        assert!(
            lines
                .iter()
                .flat_map(|l| &l.spans)
                .any(|s| s.content == "-old" && s.style.fg == Some(RED))
        );
        assert!(
            lines
                .iter()
                .flat_map(|l| &l.spans)
                .any(|s| s.content == "+new" && s.style.fg == Some(JADE))
        );
    }
    #[test]
    fn styled_wrapping_keeps_grapheme_clusters_and_words() {
        let content = vec![
            span("中文 👩‍💻 e\u{301} together ", style(FG)),
            span("boldwords", style(JADE)),
        ];
        let lines = wrap(&content, 12, false);
        let text = plain(&lines);
        assert!(text.contains("👩‍💻") && text.contains("e\u{301}"));
        assert!(text.contains("together") && text.contains("boldwords"));
        assert!(lines.iter().all(|l| l.width() <= 12));
    }
    #[test]
    fn tables_fit_the_terminal_and_stack_complete_cells_on_narrow_widths() {
        let source = "| Name | Count | State |\n|:--|--:|:--:|\n| 中文 | 486 | Ready |\n| Long name | 2 | Pending |";
        for width in [8, 20, 48] {
            let lines = markdown(source, width);
            assert!(
                lines.iter().all(|l| l.width() <= width),
                "{width}: {}",
                plain(&lines)
            );
            let text = plain(&lines);
            assert!(text.contains("中文") && text.contains("486"));
        }
    }
    #[test]
    fn nested_blocks_and_long_code_labels_stay_within_narrow_layouts() {
        let source = "> > 1. **Nested item** with 中文 content\n> >\n> > ---\n> >\n> > ```a-very-long-language-name\n> >     literal_text\n> > ```\n\n| One | Two |\n|---|---|\n| a | b |";
        for width in [4, 8, 16] {
            let lines = markdown(source, width);
            assert!(
                lines.iter().all(|line| line.width() <= width),
                "{width}: {}",
                plain(&lines)
            );
        }
    }
    #[test]
    fn escaped_entities_html_and_incomplete_streams_are_inert_text() {
        let lines = markdown(
            "<script>alert(1)</script>\n\n&#27;[31m **unfinished\n\n```sh\n    echo safe",
            50,
        );
        let text = plain(&lines);
        assert!(!text.contains('\u{1b}'));
        assert!(text.contains("<script>alert(1)</script>") && text.contains("echo safe"));
        assert!(lines.iter().all(|l| l.width() <= 50));
    }
}
