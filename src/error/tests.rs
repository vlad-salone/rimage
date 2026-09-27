use super::*;
use crate::limits::{Binding, LimitViolation, ViolationKind};

fn jpeg_path() -> PathBuf {
    PathBuf::from("/tmp/photo.jpg")
}

fn decode_failure() -> ImageErrors {
    // Mirrors the trailing newline `ImageErrors` produces for real failures.
    ImageErrors::ImageDecodeErrors("unexpected end of file".to_string())
}

fn limit_violation() -> LimitViolation {
    LimitViolation {
        kind: ViolationKind::Pixels,
        actual: 4_290_250_000,
        allowed: 1_908_874_353,
        binding: Binding::Memory,
    }
}

#[test]
fn decoder_not_implemented_becomes_an_unsupported_format_input_error() {
    let error = ImageErrors::ImageDecoderNotImplemented(zune_image::codecs::ImageFormat::Unknown);

    let classified = classify_input(&jpeg_path(), &error, None);

    assert_eq!(classified.direction(), Direction::Input);
    assert_eq!(classified.kind(), "input.unsupported-format");
    // The format comes from the extension, because the decoder never reported one.
    let message = classified.to_string();
    assert!(message.contains("jpeg"), "{message}");
    assert!(message.contains("no decoder"), "{message}");
}

#[test]
fn missing_decoder_feature_is_distinguished_from_a_missing_decoder() {
    let error = ImageErrors::ImageDecoderNotIncluded(zune_image::codecs::ImageFormat::JPEG);

    let classified = classify_input(&jpeg_path(), &error, None);
    let message = classified.to_string();

    assert!(message.contains("build time"), "{message}");
    assert_eq!(classified.kind(), "input.unsupported-format");
}

#[test]
fn a_resize_refusal_is_an_input_error_not_a_decode_error() {
    let error = ImageErrors::ImageOperationNotImplemented("resize", depth_of_first_supported());

    let classified = classify_input(
        &jpeg_path(),
        &error,
        Some(("the result does not fit in 32 bits", (70_000, 70_000))),
    );

    assert_eq!(classified.kind(), "input.invalid-resize");
    let message = classified.to_string();
    assert!(message.contains("70000x70000"), "{message}");
    assert!(message.contains("32 bits"), "{message}");
}

#[test]
fn a_resize_refusal_without_context_falls_back_to_a_decode_error() {
    // Dropping the caller's context must not lose the error entirely.
    let error = ImageErrors::ImageOperationNotImplemented("resize", depth_of_first_supported());

    let classified = classify_input(&jpeg_path(), &error, None);

    assert_eq!(classified.kind(), "input.decode");
}

#[test]
fn a_generic_decode_failure_names_the_file_and_format() {
    let classified = classify_input(&jpeg_path(), &decode_failure(), None);

    assert_eq!(classified.kind(), "input.decode");
    let message = classified.to_string();
    assert!(message.contains("photo.jpg"), "{message}");
    assert!(message.contains("jpeg"), "{message}");
    assert!(message.contains("unexpected end of file"), "{message}");
}

#[test]
fn the_message_does_not_end_in_a_stray_newline() {
    // `ImageErrors`' Display adds a trailing newline; ours must not inherit it,
    // or every log line gains a blank line after it.
    let classified = classify_input(&jpeg_path(), &decode_failure(), None);

    assert!(!classified.to_string().ends_with('\n'));
}

#[test]
fn an_unknown_extension_is_reported_as_a_generic_image() {
    let error = decode_failure();
    let classified = classify_input(Path::new("mystery.qoi"), &error, None);

    assert!(classified.to_string().contains("image"), "{classified}");
}

#[test]
fn classify_output_routes_an_encode_error_with_the_path_extension_format() {
    let error = ImageErrors::EncodeErrors(ImgEncodeErrors::ImageEncodeErrors(
        "webp encoding failed".to_string(),
    ));

    // The output path extension is authoritative: the encoder picks it, so the
    // format tag is reliable even though the error value does not carry it.
    let classified = classify_output(&jpeg_path(), &error).unwrap();

    assert_eq!(classified.direction(), Direction::Output);
    assert_eq!(classified.kind(), "output.encode");
    let message = classified.to_string();
    assert!(message.contains("photo.jpg"), "{message}");
    assert!(message.contains("jpeg"), "{message}");
    assert!(message.contains("webp encoding failed"), "{message}");
}

#[test]
fn classify_output_routes_an_io_error_to_output_io() {
    let error = ImageErrors::IoError(std::io::Error::new(
        std::io::ErrorKind::StorageFull,
        "No space left on device",
    ));

    let classified = classify_output(&jpeg_path(), &error).unwrap();

    assert_eq!(classified.kind(), "output.io");
    assert!(classified.to_string().contains("photo.jpg"));
}

#[test]
fn output_encode_error_uses_the_encoder_name_for_the_format_tag() {
    let error = ImageErrors::EncodeErrors(ImgEncodeErrors::ImageEncodeErrors(
        "encode failed".to_string(),
    ));

    // `mozjpeg` writes `.jpg`, so the format tag is `jpeg` even though the
    // encoder name does not contain "jpeg" literally.
    let classified = output_encode_error(&jpeg_path(), "mozjpeg", &error);

    assert_eq!(classified.kind(), "output.encode");
    let message = classified.to_string();
    assert!(message.contains("jpeg"), "{message}");
    assert!(message.contains("encode failed"), "{message}");
}

#[test]
fn output_io_error_preserves_the_io_kind_for_hint_routing() {
    let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");

    let classified = output_io_error(&PathBuf::from("readonly/out.png"), &error);

    assert_eq!(classified.kind(), "output.io");
    assert_eq!(classified.path(), Path::new("readonly/out.png"));
    // PermissionDenied errors get a hint, so the user knows what to fix.
    assert!(classified.hint().unwrap().contains("permission"));
}

#[test]
fn output_io_error_without_a_specific_kind_has_no_hint() {
    let error = std::io::Error::other("disk on fire");

    let classified = output_io_error(&PathBuf::from("out.png"), &error);

    assert_eq!(classified.kind(), "output.io");
    assert!(classified.hint().is_none());
}

#[test]
fn output_io_error_storage_full_has_a_hint() {
    let error = std::io::Error::new(std::io::ErrorKind::StorageFull, "disk full");

    let classified = output_io_error(&PathBuf::from("out.png"), &error);

    assert_eq!(classified.kind(), "output.io");
    let hint = classified.hint().expect("StorageFull must produce a hint");
    assert!(hint.contains("free up space"), "{hint}");
}

#[cfg(feature = "svg")]
#[test]
fn an_svg_size_limit_marker_becomes_a_structured_size_limit() {
    use crate::codecs::svg::SIZE_LIMIT_MARKER;

    let path = PathBuf::from("poster.svg");
    // Mirrors `codecs::svg::decoder::size_limit_error` shape exactly.
    let error = ImageErrors::ImageDecodeErrors(format!(
        "{SIZE_LIMIT_MARKER}132778x5000:663890000:268435456: SVG target size 132778x5000 \
         (663890000 pixels) exceeds the limit of 268435456 pixels, reduce the --resize target \
         or the intrinsic size",
    ));

    let classified = classify_input(&path, &error, None);

    match classified {
        RimageError::Input(InputError::SizeLimit {
            format,
            dimensions,
            violation,
            ..
        }) => {
            assert_eq!(format, ImageFormatId::Svg);
            assert_eq!(dimensions, Some((132_778, 5_000)));
            assert_eq!(violation.kind, ViolationKind::Pixels);
            assert_eq!(violation.actual, 663_890_000);
            assert_eq!(violation.allowed, 268_435_456);
            // The SVG render target is always bounded by the memory budget the
            // pipeline derived, so the violation is honestly labelled `Memory`
            // — never `Format` (SVG has no fixed dimension cap) and never
            // `Disk` (free space is the output volume's job).
            assert_eq!(violation.binding, Binding::Memory);
        }
        other => panic!("expected Input::SizeLimit, got {other:?}"),
    }

    // The marker prefix must not leak into the human-readable text.
    let rendered = classified.to_string();
    assert!(!rendered.contains(SIZE_LIMIT_MARKER), "{rendered}");
    assert!(classified.kind() == "input.size-limit");
}

#[cfg(feature = "svg")]
#[test]
fn an_svg_decode_error_without_the_marker_still_routes_to_decode() {
    // Without the marker the classifier cannot know the failure is a size
    // limit, so the safer default is to surface the upstream message under
    // `input.decode` rather than guess.
    let path = PathBuf::from("poster.svg");
    let error = ImageErrors::ImageDecodeErrors("Unable to parse SVG - oops".to_string());

    let classified = classify_input(&path, &error, None);
    assert!(matches!(
        classified,
        RimageError::Input(InputError::Decode { .. })
    ));
    assert_eq!(classified.kind(), "input.decode");
}

#[test]
fn a_size_limit_violation_explains_which_ceiling_bound() {
    let error = RimageError::Input(InputError::SizeLimit {
        path: PathBuf::from("panorama.png"),
        format: ImageFormatId::Png,
        dimensions: Some((65_500, 65_500)),
        violation: limit_violation(),
    });

    let message = error.to_string();
    assert!(message.contains("too large"), "{message}");
    assert!(message.contains("pixel count"), "{message}");
    assert!(message.contains("65500x65500"), "{message}");
    // The binding is what makes the message actionable: it says *why* this
    // limit and not a format constant.
    assert!(message.contains("available memory budget"), "{message}");
}

#[test]
fn every_violation_kind_is_described() {
    for kind in [
        ViolationKind::Width,
        ViolationKind::Height,
        ViolationKind::Pixels,
    ] {
        let error = RimageError::Input(InputError::SizeLimit {
            path: PathBuf::from("x.png"),
            format: ImageFormatId::Png,
            dimensions: None,
            violation: LimitViolation {
                kind,
                actual: 200,
                allowed: 100,
                binding: Binding::Format,
            },
        });

        assert!(
            !error.to_string().is_empty(),
            "{kind:?} produced no message"
        );
        assert!(error.hint().is_some(), "{kind:?} produced no hint");
    }
}

#[test]
fn out_of_space_names_both_figures() {
    let error = RimageError::Output(OutputError::OutOfSpace {
        path: PathBuf::from("out.png"),
        format: ImageFormatId::Png,
        needed: 3 * 1024 * 1024 * 1024,
        available: 512 * 1024 * 1024,
    });

    let message = error.to_string();
    assert!(message.contains("3.0 GiB"), "{message}");
    assert!(message.contains("512.0 MiB"), "{message}");

    let hint = error.hint().unwrap();
    assert!(hint.contains("2.5 GiB"), "{hint}");
}

#[test]
fn io_errors_report_the_path_that_failed() {
    let error = RimageError::Output(OutputError::Io {
        path: PathBuf::from("readonly/out.png"),
        cause: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied"),
    });

    assert_eq!(error.direction(), Direction::Output);
    assert_eq!(error.path(), Path::new("readonly/out.png"));
    assert!(error.to_string().contains("readonly/out.png"));
    assert!(error.hint().unwrap().contains("permission"));
}

#[test]
fn a_missing_input_points_at_the_path() {
    let error = RimageError::Input(InputError::Open {
        path: PathBuf::from("gone.jpg"),
        cause: std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"),
    });

    assert_eq!(error.direction(), Direction::Input);
    assert!(error.hint().unwrap().contains("exists"));
}

#[test]
fn err_kinds_are_unique_per_variant() {
    let errors = [
        RimageError::Input(InputError::Open {
            path: PathBuf::from("a"),
            cause: std::io::Error::other("x"),
        }),
        RimageError::Input(InputError::Decode {
            path: PathBuf::from("a"),
            format: ImageFormatId::Png,
            cause: decode_failure(),
        }),
        RimageError::Input(InputError::UnsupportedFormat {
            path: PathBuf::from("a"),
            format: ImageFormatId::Png,
            reason: UnsupportedReason::NotImplemented,
        }),
        RimageError::Input(InputError::SizeLimit {
            path: PathBuf::from("a"),
            format: ImageFormatId::Png,
            dimensions: None,
            violation: limit_violation(),
        }),
        RimageError::Input(InputError::InvalidResize {
            requested: (1, 1),
            reason: "x",
        }),
        RimageError::Input(InputError::Configuration {
            path: PathBuf::from("a"),
            format: ImageFormatId::Jpeg,
            cause: decode_failure(),
        }),
        RimageError::Output(OutputError::Io {
            path: PathBuf::from("a"),
            cause: std::io::Error::other("x"),
        }),
        RimageError::Output(OutputError::Encode {
            path: PathBuf::from("a"),
            format: ImageFormatId::Png,
            cause: decode_failure(),
        }),
        RimageError::Output(OutputError::SizeLimit {
            path: PathBuf::from("a"),
            format: ImageFormatId::Png,
            violation: limit_violation(),
        }),
        RimageError::Output(OutputError::OutOfSpace {
            path: PathBuf::from("a"),
            format: ImageFormatId::Png,
            needed: 1,
            available: 0,
        }),
    ];

    let mut kinds: Vec<&str> = errors.iter().map(|e| e.kind()).collect();
    let total = kinds.len();
    kinds.sort_unstable();
    kinds.dedup();
    assert_eq!(kinds.len(), total, "kind slugs collide");
}

#[test]
fn source_preserves_the_upstream_error() {
    let error = RimageError::Input(InputError::Decode {
        path: jpeg_path(),
        format: ImageFormatId::Jpeg,
        cause: decode_failure(),
    });

    // The wrapper has to keep the original reachable, or callers that matched
    // on `ImageErrors` lose their ability to do so.
    assert!(std::error::Error::source(&error).is_some());

    // Errors with no underlying OS or decoder cause report none.
    let error = RimageError::Output(OutputError::OutOfSpace {
        path: PathBuf::from("a"),
        format: ImageFormatId::Png,
        needed: 1,
        available: 0,
    });
    assert!(std::error::Error::source(&error).is_none());
}

#[test]
fn display_matches_what_displaying_the_inner_variants_produces() {
    let error = InputError::Decode {
        path: jpeg_path(),
        format: ImageFormatId::Jpeg,
        cause: decode_failure(),
    };

    assert_eq!(
        error.to_string(),
        RimageError::Input(clone_input(&error)).to_string()
    );
}

#[test]
fn hints_are_absent_when_there_is_nothing_actionable() {
    let error = RimageError::Output(OutputError::Io {
        path: PathBuf::from("out.png"),
        cause: std::io::Error::other("disk on fire"),
    });

    assert!(error.hint().is_none());
}

#[test]
fn human_bytes_uses_binary_units() {
    assert_eq!(human_bytes(0), "0 B");
    assert_eq!(human_bytes(1023), "1023 B");
    assert_eq!(human_bytes(1024), "1.0 KiB");
    assert_eq!(human_bytes(1536), "1.5 KiB");
    assert_eq!(human_bytes(1024 * 1024), "1.0 MiB");
    assert_eq!(human_bytes(1024 * 1024 * 1024), "1.0 GiB");
}

#[test]
fn human_count_switches_to_words_for_large_values() {
    assert_eq!(human_count(999), "999");
    assert_eq!(human_count(1_500_000), "1.5 million");
    assert_eq!(human_count(4_290_250_000), "4.3 billion");
}

/// The cheapest `BitType` that the resize path reports as unsupported, used
/// only to build an `ImageOperationNotImplemented` value in tests.
fn depth_of_first_supported() -> zune_core::bit_depth::BitType {
    zune_core::bit_depth::BitType::U8
}

#[test]
fn a_size_limit_constructor_names_the_path_and_format() {
    let error = input_size_limit(
        &jpeg_path(),
        ImageFormatId::Jpeg,
        Some((65_500, 65_500)),
        limit_violation(),
    );

    assert_eq!(error.direction(), Direction::Input);
    assert_eq!(error.kind(), "input.size-limit");
    assert_eq!(error.path(), jpeg_path());

    let text = error.to_string();
    assert!(text.contains("too large"), "unexpected message: {text}");
    assert!(text.contains("65500x65500"), "unexpected message: {text}");
}

#[test]
fn an_output_size_limit_constructor_reports_the_output_side() {
    let error = output_size_limit(&jpeg_path(), ImageFormatId::Jpeg, limit_violation());

    assert_eq!(error.direction(), Direction::Output);
    assert_eq!(error.kind(), "output.size-limit");
}

#[test]
fn every_error_renders_its_hint_on_a_following_line() {
    // The hint is separate from the message line so a log capture can grep one
    // line per failure and still show the advice.
    let error = input_size_limit(
        &jpeg_path(),
        ImageFormatId::Jpeg,
        Some((65_500, 65_500)),
        limit_violation(),
    );

    let hint = error
        .hint()
        .expect("a size violation always suggests a fix");
    assert!(
        hint.contains("shrink"),
        "hint should name the action: {hint}"
    );
    // The message itself must not already contain the hint, or `log` would
    // print the advice twice.
    assert!(
        !error.to_string().contains(&hint),
        "the hint must not be duplicated in the message"
    );
}

#[test]
fn an_error_without_a_hint_still_formats() {
    let error = RimageError::Output(OutputError::Io {
        path: PathBuf::from("out.png"),
        cause: std::io::Error::other("disk on fire"),
    });

    assert!(error.hint().is_none());
    assert_eq!(error.kind(), "output.io");
}

#[test]
fn input_open_error_classifies_io_failures_on_the_input_side() {
    let error = input_open_error(
        &jpeg_path(),
        &std::io::Error::new(std::io::ErrorKind::NotFound, "gone"),
    );

    assert_eq!(error.direction(), Direction::Input);
    assert_eq!(error.kind(), "input.open");
    assert_eq!(error.path(), jpeg_path());
    assert!(error.hint().unwrap().contains("exists"));
}

#[test]
fn input_config_error_tags_the_format_from_the_encoder_name() {
    let cause = ImageErrors::GenericString("Unsupported mozjpeg colorspace: neon".to_string());

    let error = input_config_error(&jpeg_path(), "mozjpeg", &cause);

    assert_eq!(error.direction(), Direction::Input);
    assert_eq!(error.kind(), "input.configuration");
    let message = error.to_string();
    assert!(message.contains("photo.jpg"), "{message}");
    assert!(message.contains("jpeg"), "{message}");
    assert!(message.contains("neon"), "{message}");
    // The cause already explains the bad value, so no generic hint is added.
    assert!(error.hint().is_none());
}

#[test]
fn input_operation_error_routes_resize_failures_to_invalid_resize() {
    let error = ImageErrors::ImageOperationNotImplemented("resize", depth_of_first_supported());

    let classified = input_operation_error(&jpeg_path(), &error);

    assert_eq!(classified.kind(), "input.decode");
    // Without caller context the resize failure degrades to a decode label,
    // but the error is not lost.
    assert!(classified.to_string().contains("photo.jpg"));
}

#[test]
fn a_configuration_error_preserves_its_cause_as_source() {
    let cause = ImageErrors::GenericString("bad value".to_string());

    let error = input_config_error(&jpeg_path(), "mozjpeg", &cause);

    assert!(std::error::Error::source(&error).is_some());
}
