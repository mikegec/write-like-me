use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: Option<String>,
}

#[derive(Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
}

#[derive(Clone)]
pub struct AnthropicClient {
    client: Client,
    api_key: String,
}

impl AnthropicClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    async fn call_with_tokens(&self, system: &str, messages: Vec<(&str, &str)>, max_tokens: u32) -> Result<String, String> {
        let msgs: Vec<Message> = messages
            .into_iter()
            .map(|(role, content)| Message {
                role: role.to_string(),
                content: content.to_string(),
            })
            .collect();

        let request = ApiRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens,
            system: system.to_string(),
            messages: msgs,
        };

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {e}"))?;

        if !status.is_success() {
            return Err(format!("API error ({status}): {body}"));
        }

        let parsed: ApiResponse =
            serde_json::from_str(&body).map_err(|e| format!("Failed to parse response: {e}"))?;

        parsed
            .content
            .into_iter()
            .find_map(|b| b.text)
            .ok_or_else(|| "No text in response".to_string())
    }

    pub async fn generate_question(
        &self,
        previous_qa: &[(String, String)],
        question_number: usize,
    ) -> Result<String, String> {
        let system = r#"You are a writing style analyst. Your job is to ask questions that elicit natural, authentic writing from the user. You want to capture how they ACTUALLY write — their vocabulary, sentence structure, punctuation habits, capitalization patterns, use of slang, emoji usage, paragraph length, and overall voice.

Ask varied questions that provoke different kinds of responses:
- Casual opinions and rants
- Technical explanations
- Storytelling and anecdotes
- Emotional responses
- Short quips and reactions
- Longer thoughtful responses
- Persuasive arguments
- Instructions or how-tos

IMPORTANT: Output ONLY the question text. No preamble, no numbering, no quotes. Just the question."#;

        if previous_qa.is_empty() {
            self.call_with_tokens(system, vec![("user", "Generate the first question to ask someone so I can analyze their writing style. Start with something casual and easy to answer.")], 1024).await
        } else {
            let mut conversation = String::new();
            for (q, a) in previous_qa.iter().rev().take(10).rev() {
                conversation.push_str(&format!("Q: {q}\nA: {a}\n\n"));
            }
            let prompt = format!(
                "Here are the previous questions and answers (showing last {} of {question_number}):\n\n{conversation}\nGenerate the next question. Vary the topic and the kind of response you're trying to elicit. Don't repeat themes. Try to get a different LENGTH and STYLE of response than recent ones.",
                previous_qa.len().min(10)
            );
            self.call_with_tokens(system, vec![("user", &prompt)], 1024).await
        }
    }

    pub async fn analyze_style(
        &self,
        samples: &[(String, String)],
    ) -> Result<String, String> {
        let system = r#"You are an expert linguist and writing style analyst. You will receive a collection of writing samples from a single person. Your job is to produce an extremely detailed, actionable writing style profile.

Analyze EVERY aspect of their writing:

1. VOCABULARY & WORD CHOICE
   - Formality level (casual/formal/mixed)
   - Favorite words and phrases they repeat
   - Slang, colloquialisms, or jargon they use
   - Words they notably avoid
   - Technical vs. plain language preference

2. SENTENCE STRUCTURE
   - Average sentence length (short/medium/long)
   - Simple vs. compound vs. complex sentences
   - Fragment usage
   - Run-on tendencies
   - How they start sentences

3. PUNCTUATION & FORMATTING
   - Comma usage (oxford comma? over/under use?)
   - Dash usage (em dash, en dash, hyphens)
   - Exclamation marks frequency
   - Question mark patterns
   - Ellipsis usage
   - Capitalization patterns (all caps for emphasis? lowercase everything?)
   - Emoji usage (which ones, how often)
   - Parenthetical asides

4. PARAGRAPH & STRUCTURE
   - Paragraph length preference
   - How they transition between ideas
   - List usage (bullets, numbered, inline)
   - How they open and close messages

5. TONE & VOICE
   - Overall attitude (enthusiastic, dry, sarcastic, earnest)
   - Humor style
   - Level of directness
   - How they express agreement/disagreement
   - Hedging language ("I think", "maybe", "probably")
   - Confidence markers

6. QUIRKS & PATTERNS
   - Typo patterns (consistent misspellings, autocorrect artifacts)
   - Abbreviations they use
   - How they handle contractions
   - Any unique verbal tics or catchphrases
   - How they emphasize things (bold, caps, italics, repetition)

Output this as a comprehensive style guide that could be used to replicate this person's writing voice. Be specific — use direct quotes from the samples as evidence. The guide should be detailed enough that someone could read it and write convincingly in this person's style."#;

        let mut sample_text = String::new();
        for (i, (q, a)) in samples.iter().enumerate() {
            sample_text.push_str(&format!("--- Sample {} ---\nPrompt: {q}\nResponse: {a}\n\n", i + 1));
        }

        let prompt = format!(
            "Here are {} writing samples from a single person. Analyze their writing style in exhaustive detail.\n\n{sample_text}",
            samples.len()
        );

        self.call_with_tokens(system, vec![("user", &prompt)], 8192).await
    }
}
