/*!
 * Markdown Renderer using termimad
 */

use termimad::MadSkin;

/// Markdown renderer with termimad
pub struct MarkdownRenderer {
    skin: MadSkin,
    buffer: String,
}

impl MarkdownRenderer {
    /// Create a new markdown renderer
    pub fn new() -> Self {
        Self {
            skin: MadSkin::default(),
            buffer: String::new(),
        }
    }

    /// Process and render a chunk of text
    pub fn process_chunk(&mut self, chunk: &str) -> String {
        self.buffer.push_str(chunk);

        // Check if we have complete lines to render
        if let Some(last_newline) = self.buffer.rfind('\n') {
            let to_render = &self.buffer[..=last_newline];
            let remaining = self.buffer[last_newline + 1..].to_string();

            // Render the complete lines
            let rendered = self.render_text(to_render);

            // Keep the incomplete line for next chunk
            self.buffer = remaining;

            rendered
        } else {
            // No complete line yet, return empty
            String::new()
        }
    }

    /// Render markdown text to terminal string
    fn render_text(&self, text: &str) -> String {
        let mut output = Vec::new();

        // Use termimad to render to buffer
        if self.skin.write_text_on(&mut output, text).is_err() {
            // Fallback to raw text if rendering fails
            return text.to_string();
        }

        String::from_utf8_lossy(&output).to_string()
    }

    /// Flush any remaining content in the buffer
    pub fn flush(&mut self) -> String {
        if self.buffer.is_empty() {
            return String::new();
        }

        let text = self.buffer.clone();
        self.buffer.clear();
        self.render_text(&text)
    }
}

impl Default for MarkdownRenderer {
    fn default() -> Self {
        Self::new()
    }
}
