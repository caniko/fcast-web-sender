//! Deterministic HLS and DASH manifest rewriting for local FCompanion URLs.
//!
//! This is deliberately a narrow media-manifest transform. It does not parse
//! or reproduce encrypted-media key material; manifests containing DRM key or
//! content-protection declarations are rejected rather than weakened.

use thiserror::Error;
use url::Url;

#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum RewriteError {
    #[error("manifest contains encrypted HLS media")]
    EncryptedHls,
    #[error("manifest contains DASH content protection")]
    ContentProtection,
    #[error("manifest URL is not an HTTP(S) URL: {value}")]
    InvalidUrl {
        value: String,
        #[source]
        source: Option<url::ParseError>,
    },
    #[error("manifest has an unterminated attribute")]
    UnterminatedAttribute,
    #[error("manifest has an unterminated BaseURL element")]
    UnterminatedBaseUrl,
}

pub fn rewrite_hls<F>(
    manifest: &str,
    source: &Url,
    mut rewrite_url: F,
) -> Result<String, RewriteError>
where
    F: FnMut(&Url) -> String,
{
    let mut output = String::with_capacity(manifest.len() + 256);
    for line in manifest.split_inclusive('\n') {
        let without_newline = line.trim_end_matches(['\r', '\n']);
        if without_newline.starts_with("#EXT-X-KEY")
            || without_newline.starts_with("#EXT-X-SESSION-KEY")
        {
            return Err(RewriteError::EncryptedHls);
        }
        let mut transformed = without_newline.to_owned();
        if !without_newline.starts_with('#') && !without_newline.trim().is_empty() {
            transformed = rewrite_url_line(without_newline, source, &mut rewrite_url)?;
        } else if without_newline.starts_with('#') {
            transformed = rewrite_attributes(without_newline, source, &["URI"], &mut rewrite_url)?;
        }
        output.push_str(&transformed);
        output.push_str(&line[without_newline.len()..]);
    }
    if !manifest.ends_with(['\n', '\r']) && output.is_empty() {
        return Ok(String::new());
    }
    Ok(output)
}

pub fn rewrite_dash<F>(
    manifest: &str,
    source: &Url,
    mut rewrite_url: F,
) -> Result<String, RewriteError>
where
    F: FnMut(&Url) -> String,
{
    if manifest.contains("<ContentProtection") || manifest.contains(":ContentProtection") {
        return Err(RewriteError::ContentProtection);
    }
    let mut output = manifest.to_owned();
    for tag in ["BaseURL", "baseURL"] {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let mut cursor = 0;
        while let Some(open_offset) = output[cursor..].find(&open) {
            let start = cursor + open_offset + open.len();
            let Some(close_offset) = output[start..].find(&close) else {
                return Err(RewriteError::UnterminatedBaseUrl);
            };
            let end = start + close_offset;
            let contents = &output[start..end];
            let value_start = start + contents.len() - contents.trim_start().len();
            let value_end = start + contents.trim_end().len();
            if value_start < value_end {
                let url = resolve_media_url(source, &output[value_start..value_end])?;
                let replacement = rewrite_url(&url);
                output.replace_range(value_start..value_end, &replacement);
                cursor = value_start + replacement.len();
            } else {
                cursor = end + close.len();
            }
        }
    }
    output = rewrite_attributes(
        &output,
        source,
        &["media", "initialization", "sourceURL"],
        &mut rewrite_url,
    )?;
    Ok(output)
}

fn rewrite_url_line<F>(
    line: &str,
    source: &Url,
    rewrite_url: &mut F,
) -> Result<String, RewriteError>
where
    F: FnMut(&Url) -> String,
{
    let leading = &line[..line.len() - line.trim_start().len()];
    let trailing = &line[line.trim_end().len()..];
    let value = line.trim();
    let url = resolve_media_url(source, value)?;
    Ok(format!("{leading}{}{trailing}", rewrite_url(&url)))
}

fn rewrite_attributes<F>(
    input: &str,
    source: &Url,
    names: &[&str],
    rewrite_url: &mut F,
) -> Result<String, RewriteError>
where
    F: FnMut(&Url) -> String,
{
    let mut output = input.to_owned();
    for name in names {
        let mut search_from = 0;
        while let Some(found) = output[search_from..].find(name) {
            let key_start = search_from + found;
            let before = output[..key_start].chars().next_back();
            if before.is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
            {
                search_from = key_start + name.len();
                continue;
            }
            let equals = key_start + name.len();
            if !output[equals..].starts_with('=') {
                search_from = equals;
                continue;
            }
            let quote_position = equals + 1;
            let quote = output[quote_position..].chars().next().unwrap_or_default();
            if quote != '"' && quote != '\'' {
                search_from = quote_position;
                continue;
            }
            let value_start = quote_position + 1;
            let Some(relative_end) = output[value_start..].find(quote) else {
                return Err(RewriteError::UnterminatedAttribute);
            };
            let value_end = value_start + relative_end;
            let value = output[value_start..value_end].to_owned();
            let url = resolve_media_url(source, &value)?;
            let replacement = rewrite_url(&url);
            output.replace_range(value_start..value_end, &replacement);
            search_from = value_start + replacement.len() + 1;
        }
    }
    Ok(output)
}

fn resolve_media_url(source: &Url, value: &str) -> Result<Url, RewriteError> {
    let url = source
        .join(value.trim())
        .map_err(|source| RewriteError::InvalidUrl {
            value: value.to_owned(),
            source: Some(source),
        })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(RewriteError::InvalidUrl {
            value: value.to_owned(),
            source: None,
        });
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapper(url: &Url) -> String {
        format!(
            "http://127.0.0.1:4000/resource/{}",
            url.path().trim_matches('/').replace('/', "_")
        )
    }

    #[test]
    fn hls_rewrites_segments_and_uri_attributes() {
        let source = Url::parse("https://media.example/video/master.m3u8").unwrap();
        let output = rewrite_hls(
            "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\nsegment-1.m4s\n",
            &source,
            mapper,
        )
        .unwrap();
        assert!(output.contains("resource/video_init.mp4"));
        assert!(output.contains("resource/video_segment-1.m4s"));
    }

    #[test]
    fn encrypted_manifests_fail_closed() {
        let source = Url::parse("https://media.example/video.m3u8").unwrap();
        assert_eq!(
            rewrite_hls("#EXT-X-KEY:METHOD=AES-128,URI=\"key\"\n", &source, mapper),
            Err(RewriteError::EncryptedHls)
        );
    }

    #[test]
    fn dash_rewrites_base_url_and_media_attribute() {
        let source = Url::parse("https://media.example/manifest.mpd").unwrap();
        let output = rewrite_dash(
            "<MPD><BaseURL>video/</BaseURL><SegmentTemplate media=\"chunk-$Number$.m4s\"/></MPD>",
            &source,
            mapper,
        )
        .unwrap();
        assert!(output.contains("resource/video"));
        assert!(output.contains("resource/chunk-$Number$.m4s"));
    }

    #[test]
    fn dash_preserves_unicode_and_trailing_whitespace_around_base_url() {
        let source = Url::parse("https://media.example/manifest.mpd").unwrap();
        let output = rewrite_dash("<BaseURL>\u{2003}video/ \t</BaseURL>", &source, mapper).unwrap();
        assert_eq!(
            output,
            "<BaseURL>\u{2003}http://127.0.0.1:4000/resource/video \t</BaseURL>"
        );
    }

    #[test]
    fn dash_rejects_unterminated_base_url() {
        let source = Url::parse("https://media.example/manifest.mpd").unwrap();
        assert_eq!(
            rewrite_dash("<BaseURL>video/", &source, mapper),
            Err(RewriteError::UnterminatedBaseUrl)
        );
    }

    #[test]
    fn invalid_url_preserves_parse_source() {
        let source = Url::parse("https://media.example/manifest.mpd").unwrap();
        assert!(matches!(
            resolve_media_url(&source, "http://["),
            Err(RewriteError::InvalidUrl {
                source: Some(_),
                ..
            })
        ));
    }
}
