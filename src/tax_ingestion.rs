/// IRS publication definition: number (used in URL path), human-readable name, and
/// comma-separated topic tags used for keyword-based chunk retrieval.
pub struct PubDef {
    pub number: &'static str,
    pub name: &'static str,
    pub topic_tags: &'static str,
}

/// A single parsed chunk ready for insertion into the `tax_reference` table.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedChunk {
    pub publication: String,
    pub section: String,
    pub topic_tags: String,
    pub content: String,
}

/// The 20 IRS publications ingested by the tax reference library.
/// Numbers match the path component of `https://www.irs.gov/publications/p{number}`.
pub static PUBLICATIONS: &[PubDef] = &[
    PubDef {
        number: "17",
        name: "Your Federal Income Tax",
        topic_tags: "general,income,deductions,standard_deduction",
    },
    PubDef {
        number: "334",
        name: "Tax Guide for Small Business",
        topic_tags: "small_business,schedule_c,self_employment,business_expense",
    },
    PubDef {
        number: "463",
        name: "Travel, Gift, and Car Expenses",
        topic_tags: "business_travel,vehicle,schedule_c,business_expense",
    },
    PubDef {
        number: "502",
        name: "Medical and Dental Expenses",
        topic_tags: "medical,dental,schedule_a",
    },
    PubDef {
        number: "505",
        name: "Tax Withholding and Estimated Tax",
        topic_tags: "estimated_payments,withholding,quarterly",
    },
    PubDef {
        number: "526",
        name: "Charitable Contributions",
        topic_tags: "charity,donations,schedule_a",
    },
    PubDef {
        number: "527",
        name: "Residential Rental Property",
        topic_tags: "rental,schedule_e,rental_expense",
    },
    PubDef {
        number: "533",
        name: "Self-Employment Tax",
        topic_tags: "self_employment,schedule_se,social_security,medicare",
    },
    PubDef {
        number: "535",
        name: "Business Expenses",
        topic_tags: "business_expense,deductions,schedule_c",
    },
    PubDef {
        number: "542",
        name: "Corporations",
        topic_tags: "corporation,form_1120s,s_corp",
    },
    PubDef {
        number: "544",
        name: "Sales and Other Dispositions of Assets",
        topic_tags: "capital_gains,form_4797,schedule_d,asset_sale",
    },
    PubDef {
        number: "547",
        name: "Casualties, Disasters, and Thefts",
        topic_tags: "casualties,losses,disaster",
    },
    PubDef {
        number: "550",
        name: "Investment Income and Expenses",
        topic_tags: "investment,schedule_d,dividends,interest_income",
    },
    PubDef {
        number: "551",
        name: "Basis of Assets",
        topic_tags: "basis,cost_basis,schedule_d,capital_gains",
    },
    PubDef {
        number: "560",
        name: "Retirement Plans for Small Business",
        topic_tags: "retirement,self_employment,pension",
    },
    PubDef {
        number: "587",
        name: "Business Use of Your Home",
        topic_tags: "home_office,form_8829",
    },
    PubDef {
        number: "936",
        name: "Home Mortgage Interest Deduction",
        topic_tags: "mortgage,interest,schedule_a",
    },
    PubDef {
        number: "946",
        name: "How to Depreciate Property",
        topic_tags: "depreciation,macrs,form_4562,section_179",
    },
    PubDef {
        number: "590a",
        name: "Contributions to Individual Retirement Arrangements",
        topic_tags: "ira,retirement,contributions",
    },
    PubDef {
        number: "590b",
        name: "Distributions from Individual Retirement Arrangements",
        topic_tags: "ira,retirement,distributions,rmd",
    },
];

// ── Public API ────────────────────────────────────────────────────────────────

/// Fetches an IRS publication HTML page and parses it into section chunks.
/// Returns `Err(message)` if the HTTP request fails; individual parse issues
/// are handled gracefully (empty sections are skipped).
pub fn fetch_and_parse(pub_def: &PubDef) -> Result<Vec<ParsedChunk>, String> {
    let url = format!("https://www.irs.gov/publications/p{}", pub_def.number);
    let response = ureq::get(&url)
        .set(
            "User-Agent",
            &format!("bursar/{}", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|e| format!("HTTP error fetching {url}: {e}"))?;
    let html = response
        .into_string()
        .map_err(|e| format!("Failed to read response body for {url}: {e}"))?;
    Ok(parse_html(&html, pub_def))
}

/// Parses IRS publication HTML into section chunks.
///
/// Splits at `<h2>` boundaries. Sections longer than 16,000 characters of
/// extracted text are further split at `<h3>` boundaries. Falls back to `<h3>`
/// if no `<h2>` exists, then to a single chunk if neither is found.
pub fn parse_html(html: &str, pub_def: &PubDef) -> Vec<ParsedChunk> {
    let pub_label = format!("Pub {}", pub_def.number);

    // Primary split: <h2> headings.
    let h2_sections = split_by_heading_level(html, "h2");

    if h2_sections.is_empty() {
        // Fallback: try <h3>.
        let h3_sections = split_by_heading_level(html, "h3");
        if h3_sections.is_empty() {
            // No headings found: single chunk with all text.
            let content = strip_tags(html);
            if content.trim().is_empty() {
                return vec![];
            }
            return vec![ParsedChunk {
                publication: pub_label,
                section: pub_def.name.to_string(),
                topic_tags: pub_def.topic_tags.to_string(),
                content,
            }];
        }
        return h3_sections
            .into_iter()
            .filter_map(|(heading, content_html)| {
                let content = strip_tags(&content_html);
                if content.trim().is_empty() {
                    return None;
                }
                Some(ParsedChunk {
                    publication: pub_label.clone(),
                    section: heading,
                    topic_tags: pub_def.topic_tags.to_string(),
                    content,
                })
            })
            .collect();
    }

    // Process h2 sections, splitting long ones at h3.
    let mut result = Vec::new();
    for (heading, content_html) in h2_sections {
        let content_text = strip_tags(&content_html);
        if content_text.len() > 16_000 {
            let sub = split_by_heading_level(&content_html, "h3");
            if !sub.is_empty() {
                for (sub_heading, sub_html) in sub {
                    let sub_text = strip_tags(&sub_html);
                    if sub_text.trim().is_empty() {
                        continue;
                    }
                    let section = if sub_heading.trim().is_empty() {
                        heading.clone()
                    } else {
                        format!("{heading} — {sub_heading}")
                    };
                    result.push(ParsedChunk {
                        publication: pub_label.clone(),
                        section,
                        topic_tags: pub_def.topic_tags.to_string(),
                        content: sub_text,
                    });
                }
                continue;
            }
            // No h3 found; keep oversized section as-is.
        }
        if !content_text.trim().is_empty() {
            result.push(ParsedChunk {
                publication: pub_label.clone(),
                section: heading,
                topic_tags: pub_def.topic_tags.to_string(),
                content: content_text,
            });
        }
    }

    result
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Splits HTML by `<{tag}>` headings.
/// Returns `(heading_text, following_content_html)` pairs.
/// The heading text is the stripped inner text of the heading element.
/// Content is the raw HTML from after the closing `</{tag}>` to the next opening `<{tag}`.
fn split_by_heading_level(html: &str, tag: &str) -> Vec<(String, String)> {
    let open_prefix = format!("<{tag}");
    let close_tag = format!("</{tag}>");
    let lower = html.to_lowercase();

    let mut result = Vec::new();
    let mut pos = 0;

    while let Some(rel) = lower[pos..].find(open_prefix.as_str()) {
        let h_start = pos + rel;

        // Find '>' that ends the opening tag (may have attributes, e.g. <h2 class="...">).
        let after_open_prefix = h_start + open_prefix.len();
        let gt = match lower[after_open_prefix..].find('>') {
            Some(r) => after_open_prefix + r + 1,
            None => break,
        };

        // Find the closing tag, e.g. </h2>.
        let close_start = match lower[gt..].find(close_tag.as_str()) {
            Some(r) => gt + r,
            None => break,
        };
        let after_close = close_start + close_tag.len();

        // Heading inner HTML → stripped text.
        let heading_text = strip_tags(&html[gt..close_start]);
        if heading_text.trim().is_empty() {
            pos = after_close;
            continue;
        }

        // Content: from after the closing heading tag to the start of the next same-level heading.
        let next_h = lower[after_close..]
            .find(open_prefix.as_str())
            .map(|r| after_close + r)
            .unwrap_or(html.len());

        let content_html = html[after_close..next_h].to_string();
        result.push((heading_text.trim().to_string(), content_html));

        pos = after_close;
    }

    result
}

/// Extracts plain text from an HTML fragment by stripping all tags.
/// Normalizes whitespace.
fn strip_tags(html: &str) -> String {
    let fragment = scraper::Html::parse_fragment(html);
    fragment
        .root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pub() -> PubDef {
        PubDef {
            number: "999",
            name: "Test Publication",
            topic_tags: "test,general",
        }
    }

    #[test]
    fn parse_h2_headings_splits_into_chunks() {
        let html = r#"
        <html><body>
        <h2>Chapter 1</h2>
        <p>Content of chapter 1.</p>
        <h2>Chapter 2</h2>
        <p>Content of chapter 2.</p>
        </body></html>
        "#;

        let chunks = parse_html(html, &test_pub());
        assert_eq!(chunks.len(), 2, "should produce 2 chunks for 2 h2 headings");
        assert_eq!(chunks[0].section, "Chapter 1");
        assert!(
            chunks[0].content.contains("Content of chapter 1"),
            "chunk 0 content: {}",
            chunks[0].content
        );
        assert_eq!(chunks[1].section, "Chapter 2");
        assert!(
            chunks[1].content.contains("Content of chapter 2"),
            "chunk 1 content: {}",
            chunks[1].content
        );
    }

    #[test]
    fn no_headings_returns_single_chunk() {
        let html = r#"<html><body><p>Some content here.</p></body></html>"#;
        let chunks = parse_html(html, &test_pub());
        assert_eq!(chunks.len(), 1, "no headings should produce a single chunk");
        assert_eq!(chunks[0].section, "Test Publication");
        assert!(
            chunks[0].content.contains("Some content here"),
            "content: {}",
            chunks[0].content
        );
    }

    #[test]
    fn tag_stripping_removes_all_html_tags() {
        let html = r#"<html><body>
        <h2>Section</h2>
        <p>Text with <b>bold</b> and <a href="x">link</a>.</p>
        </body></html>"#;
        let chunks = parse_html(html, &test_pub());
        assert_eq!(chunks.len(), 1);
        let content = &chunks[0].content;
        assert!(
            !content.contains('<'),
            "should not contain HTML tags, got: {content}"
        );
        assert!(content.contains("bold"), "bold text should be present");
        assert!(content.contains("link"), "link text should be present");
    }

    #[test]
    fn long_section_splits_at_h3() {
        // Each paragraph is ~18 chars × 1000 = ~18,000 chars of content text > 16,000 limit.
        let long_para: String = "word ".repeat(3700); // ~18,500 chars
        let html = format!(
            "<html><body>\
             <h2>Big Section</h2>\
             <h3>Sub A</h3><p>{long_para}</p>\
             <h3>Sub B</h3><p>{long_para}</p>\
             </body></html>"
        );

        let chunks = parse_html(&html, &test_pub());
        assert!(
            chunks.len() >= 2,
            "long section should be split at h3, got {} chunks",
            chunks.len()
        );
        assert!(
            chunks.iter().any(|c| c.section.contains("Sub A")),
            "should have Sub A chunk"
        );
        assert!(
            chunks.iter().any(|c| c.section.contains("Sub B")),
            "should have Sub B chunk"
        );
    }

    #[test]
    fn h3_fallback_when_no_h2() {
        let html = r#"<html><body>
        <h3>Section Alpha</h3><p>Alpha content.</p>
        <h3>Section Beta</h3><p>Beta content.</p>
        </body></html>"#;

        let chunks = parse_html(html, &test_pub());
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].section, "Section Alpha");
        assert_eq!(chunks[1].section, "Section Beta");
    }

    #[test]
    fn publication_metadata_on_all_chunks() {
        let html = r#"<html><body><h2>Chapter</h2><p>Content.</p></body></html>"#;
        let pub_def = PubDef {
            number: "17",
            name: "Your Federal Income Tax",
            topic_tags: "general,income",
        };
        let chunks = parse_html(html, &pub_def);
        for chunk in &chunks {
            assert_eq!(chunk.publication, "Pub 17");
            assert_eq!(chunk.topic_tags, "general,income");
        }
    }

    #[test]
    fn empty_html_returns_no_chunks() {
        let chunks = parse_html("", &test_pub());
        assert!(chunks.is_empty(), "empty html should produce no chunks");
    }

    #[test]
    fn whitespace_only_sections_are_skipped() {
        let html = "<html><body>\
                    <h2>Empty Section</h2>\
                    <h2>Real Section</h2><p>Real content.</p>\
                    </body></html>";
        let chunks = parse_html(html, &test_pub());
        assert_eq!(
            chunks.len(),
            1,
            "empty sections should be filtered out; got: {chunks:?}"
        );
        assert_eq!(chunks[0].section, "Real Section");
    }

    #[test]
    fn publications_list_has_20_entries() {
        assert_eq!(PUBLICATIONS.len(), 20);
    }

    #[test]
    fn all_publications_have_nonempty_fields() {
        for pub_def in PUBLICATIONS {
            assert!(!pub_def.number.is_empty(), "number is empty");
            assert!(
                !pub_def.name.is_empty(),
                "name is empty for {}",
                pub_def.number
            );
            assert!(
                !pub_def.topic_tags.is_empty(),
                "topic_tags empty for {}",
                pub_def.number
            );
        }
    }
}
