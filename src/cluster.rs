use std::collections::HashMap;

use chrono::NaiveDateTime;
use serde::Serialize;

pub struct PageForClustering {
    pub url: String,
    pub title: String,
    pub visited_at: NaiveDateTime,
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterPage {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Cluster {
    pub label: String,
    /// Average pairwise cosine similarity, or title-keyword overlap score.
    pub score: f32,
    pub started_at: String,
    pub ended_at: String,
    pub duration_mins: i64,
    /// Most frequent domain in the session — used for topic-level hiding.
    pub dominant_domain: String,
    pub domains: Vec<String>,
    pub pages: Vec<ClusterPage>,
}

const SESSION_GAP_MINS: i64 = 30;
const MIN_PAGES: usize = 3;
const MIN_SCORE: f32 = 0.35;

const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "of", "in", "to", "for", "with", "on", "at", "by", "from", "is", "was",
    "that", "this", "it", "as", "are", "be", "have", "has", "had", "do", "does", "did", "will",
    "would", "could", "should", "may", "might", "can", "and", "or", "but", "not", "no", "what",
    "how", "why", "when", "where", "who", "which", "more", "also", "new", "about", "its", "their",
    "your", "our", "my", "get", "use", "using", "used", "into", "than", "then", "so", "all", "one",
    "two", "just", "via",
];

pub fn find_clusters(pages: Vec<PageForClustering>, ignored_domains: &[String]) -> Vec<Cluster> {
    let sessions = split_sessions(pages);
    let mut clusters: Vec<Cluster> = sessions
        .into_iter()
        .filter(|s| s.len() >= MIN_PAGES)
        .filter_map(score_session)
        .filter(|c| c.score >= MIN_SCORE)
        .filter(|c| !ignored_domains.iter().any(|d| d == &c.dominant_domain))
        .collect();
    // Most coherent sessions first.
    clusters.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    clusters
}

fn split_sessions(mut pages: Vec<PageForClustering>) -> Vec<Vec<PageForClustering>> {
    pages.sort_by_key(|p| p.visited_at);
    let mut sessions: Vec<Vec<PageForClustering>> = Vec::new();
    let mut current: Vec<PageForClustering> = Vec::new();
    for page in pages {
        if let Some(last) = current.last() {
            let gap = page
                .visited_at
                .signed_duration_since(last.visited_at)
                .num_minutes();
            if gap > SESSION_GAP_MINS {
                sessions.push(std::mem::take(&mut current));
            }
        }
        current.push(page);
    }
    if !current.is_empty() {
        sessions.push(current);
    }
    sessions
}

fn score_session(pages: Vec<PageForClustering>) -> Option<Cluster> {
    let started_at = pages.first()?.visited_at;
    let ended_at = pages.last()?.visited_at;
    let duration_mins = ended_at.signed_duration_since(started_at).num_minutes();

    let embeddings: Vec<&Vec<f32>> = pages.iter().filter_map(|p| p.embedding.as_ref()).collect();
    let score = if embeddings.len() >= 2 {
        embedding_coherence(&embeddings)
    } else {
        title_overlap_score(&pages)
    };

    let label = extract_label(&pages);
    let dominant_domain = dominant_domain(&pages);
    let domains = extract_domains(&pages);

    let cluster_pages = pages
        .into_iter()
        .map(|p| ClusterPage {
            url: p.url,
            title: p.title,
        })
        .collect();

    Some(Cluster {
        label,
        score,
        started_at: started_at.format("%Y-%m-%dT%H:%M:%S").to_string(),
        ended_at: ended_at.format("%Y-%m-%dT%H:%M:%S").to_string(),
        duration_mins,
        dominant_domain,
        domains,
        pages: cluster_pages,
    })
}

fn embedding_coherence(embeddings: &[&Vec<f32>]) -> f32 {
    let mut total = 0.0f32;
    let mut count = 0usize;
    for i in 0..embeddings.len() {
        for j in (i + 1)..embeddings.len() {
            total += cosine_similarity(embeddings[i], embeddings[j]);
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

fn title_overlap_score(pages: &[PageForClustering]) -> f32 {
    let word_lists: Vec<Vec<String>> = pages.iter().map(|p| tokenize(&p.title)).collect();
    let mut freq: HashMap<&str, usize> = HashMap::new();
    for words in &word_lists {
        // count each word once per document (set membership)
        let unique: std::collections::HashSet<&str> = words.iter().map(|s| s.as_str()).collect();
        for w in unique {
            *freq.entry(w).or_default() += 1;
        }
    }
    // Words that appear in ≥2 documents
    let shared: usize = freq.values().filter(|&&n| n >= 2).count();
    let total_unique: usize = freq.len();
    if total_unique == 0 {
        0.0
    } else {
        shared as f32 / total_unique as f32
    }
}

fn extract_label(pages: &[PageForClustering]) -> String {
    let mut freq: HashMap<String, usize> = HashMap::new();
    for page in pages {
        let unique: std::collections::HashSet<String> = tokenize(&page.title).into_iter().collect();
        for w in unique {
            *freq.entry(w).or_default() += 1;
        }
    }
    let mut sorted: Vec<(String, usize)> = freq.into_iter().filter(|(_, n)| *n >= 2).collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let top: Vec<String> = sorted
        .into_iter()
        .take(3)
        .map(|(w, _)| capitalize(&w))
        .collect();
    if top.is_empty() {
        // Fall back to most-visited domain
        let domain = pages
            .iter()
            .filter_map(|p| host_from_url(&p.url))
            .collect::<Vec<_>>()
            .into_iter()
            .fold(HashMap::new(), |mut m, h| {
                *m.entry(h).or_insert(0usize) += 1;
                m
            })
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .map(|(h, _)| h)
            .unwrap_or("Browsing session");
        domain.to_string()
    } else {
        top.join(" / ")
    }
}

fn dominant_domain(pages: &[PageForClustering]) -> String {
    let mut freq: HashMap<String, usize> = HashMap::new();
    for page in pages {
        if let Some(h) = host_from_url(&page.url) {
            *freq.entry(h.to_string()).or_default() += 1;
        }
    }
    freq.into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(h, _)| h)
        .unwrap_or_default()
}

fn extract_domains(pages: &[PageForClustering]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    pages
        .iter()
        .filter_map(|p| host_from_url(&p.url))
        .filter(|h| seen.insert(h.to_string()))
        .take(5)
        .map(|s| s.to_string())
        .collect()
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .collect()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

fn host_from_url(url: &str) -> Option<&str> {
    let after = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let end = after
        .find(|c| matches!(c, '/' | '?' | '#'))
        .unwrap_or(after.len());
    let authority = &after[..end];
    let host = match authority.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => authority,
    };
    // strip www. prefix for cleaner domain display
    Some(host.strip_prefix("www.").unwrap_or(host))
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_page(url: &str, title: &str, mins_from_epoch: i64) -> PageForClustering {
        PageForClustering {
            url: url.to_string(),
            title: title.to_string(),
            visited_at: chrono::DateTime::from_timestamp(mins_from_epoch * 60, 0)
                .unwrap()
                .naive_utc(),
            embedding: None,
        }
    }

    #[test]
    fn session_gap_splits_correctly() {
        let pages = vec![
            make_page("https://a.com", "Rust async", 0),
            make_page("https://b.com", "Rust futures", 10),
            make_page("https://c.com", "Rust tokio", 20),
            // 60-minute gap
            make_page("https://d.com", "3D printing guide", 80),
            make_page("https://e.com", "3D printing slicer", 90),
            make_page("https://f.com", "3D printing materials", 100),
        ];
        let sessions = split_sessions(pages);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].len(), 3);
        assert_eq!(sessions[1].len(), 3);
    }

    #[test]
    fn title_overlap_score_high_for_related_titles() {
        let pages = vec![
            make_page("https://a.com", "Rust async programming", 0),
            make_page("https://b.com", "Rust async runtime", 5),
            make_page("https://c.com", "Async Rust tutorial", 10),
        ];
        let score = title_overlap_score(&pages);
        assert!(score >= 0.4, "score was {score}");
    }

    #[test]
    fn title_overlap_score_low_for_unrelated_titles() {
        let pages = vec![
            make_page("https://a.com", "Pizza recipe Italian", 0),
            make_page("https://b.com", "Rust programming language", 5),
            make_page("https://c.com", "Guitar chord theory", 10),
        ];
        let score = title_overlap_score(&pages);
        assert!(score < 0.3, "score was {score}");
    }

    #[test]
    fn label_reflects_shared_words() {
        let pages = vec![
            make_page("https://a.com", "Rust async programming guide", 0),
            make_page("https://b.com", "Rust async runtime tutorial", 5),
            make_page("https://c.com", "Async Rust programming", 10),
        ];
        let label = extract_label(&pages);
        assert!(
            label.to_lowercase().contains("rust") || label.to_lowercase().contains("async"),
            "label was: {label}"
        );
    }

    #[test]
    fn clusters_filtered_by_min_score() {
        let pages = vec![
            make_page("https://a.com", "Pizza recipe", 0),
            make_page("https://b.com", "Rust futures", 5),
            make_page("https://c.com", "Guitar theory", 10),
        ];
        let clusters = find_clusters(pages, &[]);
        assert!(
            clusters.is_empty(),
            "unrelated pages should not form a cluster"
        );
    }
}
