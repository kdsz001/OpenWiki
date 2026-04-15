use crate::storage::models::{CapturedContent, ContentType};
use crate::storage::repository::Repository;
use chrono::Local;
use std::fs;
use std::path::{Path, PathBuf};

/// Convert a date string (e.g. "2026-03-19") to an English weekday name.
fn weekday_english(date_str: &str) -> String {
    use chrono::NaiveDate;
    match NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        Ok(date) => {
            use chrono::Datelike;
            match date.weekday() {
                chrono::Weekday::Mon => "Monday".to_string(),
                chrono::Weekday::Tue => "Tuesday".to_string(),
                chrono::Weekday::Wed => "Wednesday".to_string(),
                chrono::Weekday::Thu => "Thursday".to_string(),
                chrono::Weekday::Fri => "Friday".to_string(),
                chrono::Weekday::Sat => "Saturday".to_string(),
                chrono::Weekday::Sun => "Sunday".to_string(),
            }
        }
        Err(_) => String::new(),
    }
}

fn append_grouped_content_sections(
    md: &mut String,
    contents: &[&CapturedContent],
    export_dir: &Path,
    export_images: bool,
) {
    let texts: Vec<&CapturedContent> = contents
        .iter()
        .copied()
        .filter(|c| matches!(c.content_type, ContentType::Text))
        .collect();
    let urls: Vec<&CapturedContent> = contents
        .iter()
        .copied()
        .filter(|c| matches!(c.content_type, ContentType::Url))
        .collect();
    let images: Vec<&CapturedContent> = contents
        .iter()
        .copied()
        .filter(|c| matches!(c.content_type, ContentType::Image))
        .collect();
    let mixed: Vec<&CapturedContent> = contents
        .iter()
        .copied()
        .filter(|c| matches!(c.content_type, ContentType::Mixed))
        .collect();

    if !texts.is_empty() {
        md.push_str("## Text\n\n");
        for item in &texts {
            write_content_item(md, item, export_dir, export_images);
        }
    }

    if !urls.is_empty() {
        md.push_str("## Links\n\n");
        for item in &urls {
            write_content_item(md, item, export_dir, export_images);
        }
    }

    if !images.is_empty() {
        md.push_str("## Images\n\n");
        for item in &images {
            write_content_item(md, item, export_dir, export_images);
        }
    }

    if !mixed.is_empty() {
        md.push_str("## Other\n\n");
        for item in &mixed {
            write_content_item(md, item, export_dir, export_images);
        }
    }
}

/// Generate markdown content for a single day, grouped by content type.
pub fn generate_day_markdown(
    date: &str,
    contents: &[CapturedContent],
    export_dir: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    let weekday = weekday_english(date);
    let mut md = format!("# {} {}\n\n", date, weekday);
    let refs: Vec<&CapturedContent> = contents.iter().collect();
    append_grouped_content_sections(&mut md, &refs, export_dir, true);
    Ok(md)
}

/// Generate markdown for an arbitrary selection of content items, grouped by date.
pub fn generate_selection_markdown(
    title: &str,
    contents: &[CapturedContent],
    export_dir: &Path,
    export_images: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut md = format!("# {}\n\n", title);
    md.push_str(&format!(
        "_Generated at {}_\n\n",
        Local::now().format("%Y-%m-%d %H:%M")
    ));

    let mut ordered: Vec<&CapturedContent> = contents.iter().collect();
    ordered.sort_by(|a, b| b.captured_at.cmp(&a.captured_at));

    let mut current_date = String::new();
    let mut current_group: Vec<&CapturedContent> = Vec::new();

    for item in ordered {
        let item_date = item
            .captured_at
            .get(..10)
            .unwrap_or("Unknown Date")
            .to_string();
        if current_date.is_empty() {
            current_date = item_date.clone();
        }
        if item_date != current_date {
            let weekday = weekday_english(&current_date);
            md.push_str(&format!("## {} {}\n\n", current_date, weekday));
            append_grouped_content_sections(&mut md, &current_group, export_dir, export_images);
            current_group.clear();
            current_date = item_date.clone();
        }
        current_group.push(item);
    }

    if !current_group.is_empty() {
        let weekday = weekday_english(&current_date);
        md.push_str(&format!("## {} {}\n\n", current_date, weekday));
        append_grouped_content_sections(&mut md, &current_group, export_dir, export_images);
    }

    Ok(md)
}

fn collect_contents_by_ids(
    ids: &[String],
    repo: &Repository,
) -> Result<Vec<CapturedContent>, Box<dyn std::error::Error>> {
    let mut contents = Vec::new();
    for id in ids {
        if let Some(content) = repo.get_content_by_id(id)? {
            contents.push(content);
        }
    }
    Ok(contents)
}

pub fn render_selected_markdown(
    ids: &[String],
    repo: &Repository,
    export_dir: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    let contents = collect_contents_by_ids(ids, repo)?;
    if contents.is_empty() {
        return Err("No content selected".into());
    }
    generate_selection_markdown("OpenWiki Selected Content", &contents, export_dir, false)
}

pub fn export_selected_single_file(
    ids: &[String],
    repo: &Repository,
    export_dir: &Path,
) -> Result<(PathBuf, usize), Box<dyn std::error::Error>> {
    let contents = collect_contents_by_ids(ids, repo)?;
    if contents.is_empty() {
        return Err("No content selected".into());
    }

    let selected_dir = export_dir.join("selected");
    fs::create_dir_all(&selected_dir)?;
    let filename = format!("OpenWiki-Selected-{}", Local::now().format("%Y%m%d-%H%M%S"));
    let md = generate_selection_markdown(&filename, &contents, export_dir, true)?;
    let file_path = selected_dir.join(format!("{}.md", filename));
    fs::write(&file_path, md)?;
    Ok((file_path, contents.len()))
}

/// Write a single content item to the markdown string.
fn write_content_item(
    md: &mut String,
    item: &CapturedContent,
    export_dir: &Path,
    export_images: bool,
) {
    let time = extract_time(&item.captured_at);
    md.push_str(&format!("### {} — {}\n\n", time, item.source_app));

    // Summary
    if let Some(summary) = &item.summary {
        if !summary.is_empty() {
            md.push_str(&format!("**Summary**: {}\n\n", summary));
        }
    }

    // Tags
    if let Some(tags) = &item.tags {
        if !tags.is_empty() {
            md.push_str(&format!("**Tags**: {}\n\n", tags));
        }
    }

    // Source URL (for link type)
    if let Some(url) = &item.source_url {
        if !url.is_empty() {
            md.push_str(&format!("**Link**: [{}]({})\n\n", url, url));
        }
    }

    // Image (copy file + embed / preserve original path)
    if let Some(image_path) = &item.image_path {
        let src = Path::new(image_path);
        if src.exists() {
            if export_images {
                if let Some(filename) = src.file_name() {
                    let images_dir = export_dir.join("images");
                    let _ = fs::create_dir_all(&images_dir);
                    let dest = images_dir.join(filename);
                    let _ = fs::copy(src, &dest);
                    md.push_str(&format!(
                        "![image](../images/{})\n\n",
                        filename.to_string_lossy()
                    ));
                }
            } else {
                md.push_str(&format!("**Image File**: `{}`\n\n", src.display()));
            }
        }
    }

    // Full original text / OCR text / fetched URL content
    // Prefer clean_content (AI-cleaned) over raw_text for URL content
    let display_text = item.clean_content.as_ref().or(item.raw_text.as_ref());
    if let Some(text) = display_text {
        if !text.is_empty() {
            match item.content_type {
                ContentType::Image => {
                    md.push_str("**OCR Text**:\n\n");
                    md.push_str(text);
                    md.push_str("\n\n");
                }
                ContentType::Url => {
                    md.push_str("**Original Content**:\n\n");
                    md.push_str(text);
                    md.push_str("\n\n");
                }
                _ => {
                    md.push_str(text);
                    md.push_str("\n\n");
                }
            }
        }
    }

    // User note
    if let Some(note) = &item.user_note {
        if !note.is_empty() {
            md.push_str(&format!("> **Note**: {}\n\n", note));
        }
    }

    md.push_str("---\n\n");
}

/// Extract the HH:MM time portion from an ISO-8601 datetime string.
fn extract_time(datetime: &str) -> String {
    // captured_at is like "2026-03-19T14:30:00+08:00" or "2026-03-19 14:30:00"
    if let Some(t_pos) = datetime.find('T') {
        let time_part = &datetime[t_pos + 1..];
        return time_part.chars().take(5).collect();
    }
    if let Some(space_pos) = datetime.find(' ') {
        let time_part = &datetime[space_pos + 1..];
        return time_part.chars().take(5).collect();
    }
    String::new()
}

/// Export a single day's content to a markdown file.
pub fn export_day(
    date: &str,
    repo: &Repository,
    export_dir: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let contents = repo.get_content_for_date(date)?;
    if contents.is_empty() {
        return Err(format!("No content found for date {}", date).into());
    }

    let md = generate_day_markdown(date, &contents, export_dir)?;

    // Create month subdirectory, e.g. "2026-03/"
    let month_dir_name = if date.len() >= 7 { &date[..7] } else { date };
    let month_dir = export_dir.join(month_dir_name);
    fs::create_dir_all(&month_dir)?;

    let file_path = month_dir.join(format!("{}.md", date));
    fs::write(&file_path, md)?;

    Ok(file_path)
}

/// Export all dates that have content to markdown files.
/// Returns the number of files exported.
pub fn export_all(
    repo: &Repository,
    export_dir: &Path,
) -> Result<usize, Box<dyn std::error::Error>> {
    let dates = repo.get_dates_with_content()?;
    let mut count = 0;

    for (date, _cnt) in &dates {
        match export_day(date, repo, export_dir) {
            Ok(_) => count += 1,
            Err(e) => {
                log::warn!("Failed to export date {}: {}", date, e);
            }
        }
    }

    Ok(count)
}

/// Export content for a date range (inclusive) to markdown files.
/// Returns the number of files exported.
pub fn export_date_range(
    start: &str,
    end: &str,
    repo: &Repository,
    export_dir: &Path,
) -> Result<usize, Box<dyn std::error::Error>> {
    let dates = repo.get_dates_with_content()?;
    let mut count = 0;

    for (date, _cnt) in &dates {
        if date.as_str() >= start && date.as_str() <= end {
            match export_day(date, repo, export_dir) {
                Ok(_) => count += 1,
                Err(e) => {
                    log::warn!("Failed to export date {}: {}", date, e);
                }
            }
        }
    }

    Ok(count)
}

/// Export all content into a single markdown file, grouped by date.
/// Returns the file path of the exported file.
pub fn export_all_single_file(
    repo: &Repository,
    export_dir: &Path,
) -> Result<(PathBuf, usize), Box<dyn std::error::Error>> {
    let dates = repo.get_dates_with_content()?;
    export_dates_to_single_file(&dates, repo, export_dir, "OpenWiki-All-Content")
}

/// Export a date range into a single markdown file, grouped by date.
/// Returns the file path and content count.
pub fn export_range_single_file(
    start: &str,
    end: &str,
    repo: &Repository,
    export_dir: &Path,
) -> Result<(PathBuf, usize), Box<dyn std::error::Error>> {
    let dates = repo.get_dates_with_content()?;
    let filtered: Vec<(String, i64)> = dates
        .into_iter()
        .filter(|(d, _)| d.as_str() >= start && d.as_str() <= end)
        .collect();
    let filename = format!("OpenWiki-{}-to-{}", start, end);
    export_dates_to_single_file(&filtered, repo, export_dir, &filename)
}

/// Internal: merge multiple days into one markdown file.
fn export_dates_to_single_file(
    dates: &[(String, i64)],
    repo: &Repository,
    export_dir: &Path,
    filename: &str,
) -> Result<(PathBuf, usize), Box<dyn std::error::Error>> {
    fs::create_dir_all(export_dir)?;

    let mut md = String::new();
    let mut total_items = 0usize;

    for (date, _cnt) in dates {
        let contents = match repo.get_content_for_date(date) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if contents.is_empty() {
            continue;
        }
        total_items += contents.len();
        let day_md = generate_day_markdown(date, &contents, export_dir)?;
        md.push_str(&day_md);
        md.push('\n');
    }

    if md.is_empty() {
        return Err("No content to export".into());
    }

    let file_path = export_dir.join(format!("{}.md", filename));
    fs::write(&file_path, &md)?;
    log::info!(
        "Exported {} items to single file: {}",
        total_items,
        file_path.display()
    );

    Ok((file_path, total_items))
}
