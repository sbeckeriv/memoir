use scraper::{Html, Selector};
use serde_json::Value;

use crate::recipe::RecipeCard;

/// Try to extract a recipe from schema.org JSON-LD embedded in raw HTML.
pub fn extract_recipe_schema(html: &str) -> Option<RecipeCard> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse(r#"script[type="application/ld+json"]"#).unwrap();
    for el in doc.select(&sel) {
        let text = el.text().collect::<String>();
        if let Ok(val) = serde_json::from_str::<Value>(&text)
            && let Some(card) = find_recipe(&val)
        {
            return Some(card);
        }
    }
    None
}

fn find_recipe(val: &Value) -> Option<RecipeCard> {
    if let Some(arr) = val.as_array() {
        return arr.iter().find_map(extract_recipe_object);
    }
    if let Some(graph) = val.get("@graph").and_then(|g| g.as_array()) {
        return graph.iter().find_map(extract_recipe_object);
    }
    extract_recipe_object(val)
}

fn extract_recipe_object(val: &Value) -> Option<RecipeCard> {
    let at_type = val.get("@type").and_then(|t| {
        if let Some(s) = t.as_str() {
            Some(s.to_string())
        } else if let Some(arr) = t.as_array() {
            arr.iter().find_map(|v| v.as_str().map(String::from))
        } else {
            None
        }
    })?;
    if !at_type.eq_ignore_ascii_case("Recipe") {
        return None;
    }

    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let ingredients: Vec<String> = val
        .get("recipeIngredient")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let instructions = extract_instructions(val);

    let prep_time = val
        .get("prepTime")
        .and_then(|v| v.as_str())
        .map(parse_iso_duration);
    let cook_time = val
        .get("cookTime")
        .and_then(|v| v.as_str())
        .map(parse_iso_duration);
    let total_time = val
        .get("totalTime")
        .and_then(|v| v.as_str())
        .map(parse_iso_duration);
    let prep_time = prep_time.or_else(|| total_time.clone());
    let cook_time = cook_time.or(total_time);

    let servings = val.get("recipeYield").and_then(|v| {
        if let Some(s) = v.as_str() {
            Some(s.trim().to_string())
        } else if let Some(n) = v.as_u64() {
            Some(n.to_string())
        } else if let Some(arr) = v.as_array() {
            arr.first()
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
        } else {
            None
        }
    });

    let image_url = val.get("image").and_then(|v| {
        if let Some(s) = v.as_str() {
            Some(s.to_string())
        } else if let Some(obj) = v.as_object() {
            obj.get("url").and_then(|u| u.as_str()).map(String::from)
        } else if let Some(arr) = v.as_array() {
            arr.first().and_then(|v| {
                if let Some(s) = v.as_str() {
                    Some(s.to_string())
                } else {
                    v.get("url").and_then(|u| u.as_str()).map(String::from)
                }
            })
        } else {
            None
        }
    });

    if name.is_empty() && ingredients.is_empty() && instructions.is_empty() {
        return None;
    }

    Some(RecipeCard {
        name,
        ingredients,
        instructions,
        prep_time,
        cook_time,
        servings,
        image_url,
    })
}

fn extract_instructions(val: &Value) -> Vec<String> {
    let Some(inst) = val.get("recipeInstructions") else {
        return vec![];
    };
    if let Some(s) = inst.as_str() {
        return s
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
    }
    if let Some(arr) = inst.as_array() {
        return arr
            .iter()
            .filter_map(|v| {
                if let Some(s) = v.as_str() {
                    let s = s.trim();
                    if s.is_empty() {
                        None
                    } else {
                        Some(s.to_string())
                    }
                } else if let Some(text) = v.get("text").and_then(|t| t.as_str()) {
                    let t = text.trim();
                    if t.is_empty() {
                        None
                    } else {
                        Some(t.to_string())
                    }
                } else if let Some(steps) = v.get("itemListElement").and_then(|l| l.as_array()) {
                    // HowToSection — flatten nested steps
                    let texts: Vec<String> = steps
                        .iter()
                        .filter_map(|s| s.get("text").and_then(|t| t.as_str()))
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if texts.is_empty() {
                        None
                    } else {
                        Some(texts.join(" "))
                    }
                } else {
                    None
                }
            })
            .collect();
    }
    vec![]
}

/// Parse ISO 8601 durations like PT1H30M → "1 hr 30 min".
fn parse_iso_duration(dur: &str) -> String {
    let time_part = if let Some(t_pos) = dur.find('T') {
        &dur[t_pos + 1..]
    } else {
        return dur.to_string();
    };
    let mut hours = 0u32;
    let mut mins = 0u32;
    let mut remaining = time_part;
    if let Some(h_pos) = remaining.find('H') {
        if let Ok(h) = remaining[..h_pos].parse::<u32>() {
            hours = h;
        }
        remaining = &remaining[h_pos + 1..];
    }
    if let Some(m_pos) = remaining.find('M')
        && let Ok(m) = remaining[..m_pos].parse::<u32>()
    {
        mins = m;
    }
    match (hours, mins) {
        (0, m) if m > 0 => format!("{m} min"),
        (h, 0) if h > 0 => format!("{h} hr"),
        (h, m) if h > 0 => format!("{h} hr {m} min"),
        _ => dur.to_string(),
    }
}

/// Keyword heuristic: returns true if the page text looks like it contains a recipe.
/// Used to decide whether to queue a page for LLM extraction.
pub fn looks_like_recipe(title: &str, body: &str) -> bool {
    const TITLE_KEYWORDS: &[&str] = &[
        "recipe",
        "recipes",
        "how to make",
        "how to cook",
        "how to bake",
        "baking",
        "homemade",
        "from scratch",
        "no-knead",
        "easy dough",
        "pizza dough",
        "bread dough",
    ];
    const BODY_KEYWORDS: &[&str] = &[
        "ingredient",
        "ingredients",
        "directions",
        "instructions",
        "prep time",
        "cook time",
        "cooking time",
        "preheat",
        "tablespoon",
        "teaspoon",
        "recipe",
        "serves",
        "makes",
        "flour",
        "butter",
        "cups of",
        "grams",
        "ounces",
    ];
    let lower_title = title.to_lowercase();
    let lower_body = body.to_lowercase();
    if TITLE_KEYWORDS.iter().any(|kw| lower_title.contains(kw)) {
        return true;
    }
    BODY_KEYWORDS
        .iter()
        .filter(|kw| lower_body.contains(*kw))
        .count()
        >= 2
}

/// Build the LLM prompt for extracting a recipe from page text.
pub fn recipe_extract_prompt(title: &str, body: &str) -> String {
    let body_preview = &body[..body.len().min(4000)];
    format!(
        r#"Extract the recipe from this web page.

Page title: {title}

Page text:
{body_preview}

If this page contains a recipe, return ONLY a JSON object (no other text) with this exact structure:
{{
  "name": "Recipe Name",
  "ingredients": ["2 cups flour", "1 tsp salt"],
  "instructions": ["Step 1 text", "Step 2 text"],
  "prep_time": "15 min",
  "cook_time": "30 min",
  "servings": "4 servings",
  "image_url": null
}}

If this page does not contain a recipe, return only the word: null"#
    )
}

/// Parse an LLM text response into a RecipeCard.
pub fn parse_llm_recipe(response: &str) -> Option<RecipeCard> {
    let trimmed = response.trim();
    if trimmed.eq_ignore_ascii_case("null") || trimmed.is_empty() {
        return None;
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<RecipeCard>(&trimmed[start..=end]).ok()
}

/// Extract the hostname (without port) from an http/https URL.
pub fn url_host(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..end];
    Some(match authority.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h,
        _ => authority,
    })
}
