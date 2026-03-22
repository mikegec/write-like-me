use std::fs;
use std::path::Path;

pub fn generate_skill(style_profile: &str, output_dir: &str) -> Result<String, String> {
    let skill_dir = Path::new(output_dir).join(".claude").join("skills");
    fs::create_dir_all(&skill_dir).map_err(|e| format!("Failed to create skill directory: {e}"))?;

    let skill_content = format!(
        r#"---
name: write-like-me
description: "Write text that matches the user's personal writing style. Use this skill when asked to 'write like me', 'use my voice', 'match my style', or similar requests."
---

# Write Like Me

When the user asks you to write in their style, follow this writing style profile precisely. Do NOT fall into generic LLM writing patterns. Match every quirk, pattern, and tendency described below.

## Critical Rules

1. NEVER use generic LLM phrases like "I'd be happy to", "Certainly!", "Great question!", "Let me", "Here's", "Sure!", "Absolutely!", or other AI-assistant patterns.
2. NEVER over-structure with unnecessary headers, bullet points, or numbered lists unless the style profile indicates the user does this.
3. NEVER be more formal, polished, or verbose than the style profile indicates.
4. DO match the user's typical sentence length, vocabulary level, and punctuation habits exactly.
5. DO replicate their quirks — typos, abbreviations, emoji patterns, capitalization habits, all of it.
6. DO match their emotional tone and level of directness.

## Style Profile

{style_profile}

## How to Apply

When writing in this person's style:
- Read the full style profile above before writing anything
- Draft your response, then review it against the profile
- Check: Does this sound like a human wrote it, or an AI? If AI, rewrite.
- Check: Does this match the specific patterns documented above? Adjust.
- Shorter is usually more authentic than longer — this person probably doesn't write walls of text for simple questions
"#
    );

    let skill_path = skill_dir.join("write-like-me.md");
    fs::write(&skill_path, &skill_content)
        .map_err(|e| format!("Failed to write skill file: {e}"))?;

    Ok(skill_path.to_string_lossy().to_string())
}
