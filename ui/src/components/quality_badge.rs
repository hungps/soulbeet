use dioxus::prelude::*;

/// Format a sample rate in Hz as kHz, trimming a trailing .0 (44100 -> "44.1", 48000 -> "48").
fn format_khz(hz: i32) -> String {
    let khz = hz as f64 / 1000.0;
    if khz.fract() == 0.0 {
        format!("{}", khz as i64)
    } else {
        format!("{khz:.1}")
    }
}

fn quality_label(
    quality: &str,
    bitrate: Option<i32>,
    bit_depth: Option<i32>,
    sample_rate: Option<i32>,
    backend_data: Option<&str>,
) -> String {
    let search_result = backend_data
        .and_then(|data| serde_json::from_str::<shared::slskd::SearchResult>(data).ok());

    let bitrate = bitrate.or_else(|| search_result.as_ref().and_then(|sr| sr.bitrate));
    let bit_depth = bit_depth.or_else(|| search_result.as_ref().and_then(|sr| sr.bit_depth));
    let sample_rate = sample_rate.or_else(|| search_result.as_ref().and_then(|sr| sr.sample_rate));
    let quality = quality.to_lowercase();

    if quality == "mp3" {
        match bitrate {
            Some(br) if br > 0 => format!("MP3 {br} kbps"),
            _ => "MP3".to_string(),
        }
    } else if matches!(
        quality.as_str(),
        "flac" | "wav" | "alac" | "aiff" | "aif" | "ape"
    ) {
        match (bit_depth, sample_rate) {
            (Some(depth), Some(rate)) => {
                format!("{} {depth}/{}", quality.to_uppercase(), format_khz(rate))
            }
            (Some(depth), None) => format!("{} {depth}", quality.to_uppercase()),
            (None, Some(rate)) => format!("{} {}", quality.to_uppercase(), format_khz(rate)),
            _ => quality.to_uppercase(),
        }
    } else if matches!(quality.as_str(), "m4a" | "aac" | "ogg" | "opus" | "wma") {
        match bitrate {
            Some(br) if br > 0 => format!("{} {br} kbps", quality.to_uppercase()),
            _ => quality.to_uppercase(),
        }
    } else {
        quality.to_uppercase()
    }
}

fn quality_color(quality: &str) -> &'static str {
    match quality.to_lowercase().as_str() {
        "flac" | "wav" | "alac" | "aiff" | "aif" | "ape" => {
            "bg-blue-600/30 text-blue-300 border-blue-500/40"
        }
        "m4a" | "aac" | "ogg" | "opus" => "bg-green-600/30 text-green-300 border-green-500/40",
        _ => "bg-gray-600/30 text-gray-300 border-gray-500/40",
    }
}

#[component]
pub fn QualityBadge(
    quality: String,
    #[props(default)] backend_data: Option<String>,
    #[props(default)] bitrate: Option<i32>,
    #[props(default)] bit_depth: Option<i32>,
    #[props(default)] sample_rate: Option<i32>,
    #[props(default)] label: Option<String>,
    #[props(default)] compact: bool,
) -> Element {
    let label = label.unwrap_or_else(|| {
        quality_label(
            &quality,
            bitrate,
            bit_depth,
            sample_rate,
            backend_data.as_deref(),
        )
    });
    let color = quality_color(&quality);
    let size = if compact {
        "text-[9px] px-1 py-0 leading-tight"
    } else {
        "text-[10px] px-1.5 py-0.5"
    };

    rsx! {
        span {
            class: "font-mono rounded border {size} {color} uppercase shrink-0",
            "{label}"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp3_shows_the_bitrate_when_the_search_reported_one() {
        assert_eq!(quality_label("mp3", None, None, None, None), "MP3");
        for (bitrate, label) in [
            (320, "MP3 320 kbps"),
            (256, "MP3 256 kbps"),
            (192, "MP3 192 kbps"),
            (128, "MP3 128 kbps"),
        ] {
            let data = format!(
                r#"{{"username":"u","filename":"a.mp3","size":1,"bitrate":{bitrate},"duration":1,"hasFreeUploadSlot":true,"uploadSpeed":0,"queueLength":0}}"#
            );
            assert_eq!(quality_label("mp3", None, None, None, Some(&data)), label);
        }
    }

    #[test]
    fn flac_shows_bit_depth_and_sample_rate() {
        let data = r#"{"username":"u","filename":"a.flac","size":1,"bitrate":null,"duration":1,"sampleRate":44100,"bitDepth":16,"hasFreeUploadSlot":true,"uploadSpeed":0,"queueLength":0}"#;
        assert_eq!(quality_label("flac", None, None, None, Some(data)), "FLAC 16/44.1");
        assert_eq!(quality_label("flac", None, None, None, None), "FLAC");
    }

    #[test]
    fn explicit_library_quality_wins_over_search_data() {
        assert_eq!(
            quality_label("flac", None, Some(24), Some(96000), None),
            "FLAC 24/96"
        );
        assert_eq!(
            quality_label("flac", Some(320), Some(16), Some(44100), None),
            "FLAC 16/44.1"
        );
        assert_eq!(
            quality_label("mp3", Some(320), None, None, None),
            "MP3 320 kbps"
        );
    }

    #[test]
    fn other_formats_are_their_extension_or_their_bitrate() {
        assert_eq!(quality_label("m4a", None, None, None, None), "M4A");
        assert_eq!(
            quality_label("m4a", Some(256), None, None, None),
            "M4A 256 kbps"
        );
        assert_eq!(quality_label("", None, None, None, None), "");
    }
}
