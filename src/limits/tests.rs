use super::*;

/// A budget whose address cap cannot become the thing under test.
///
/// [`ADDRESS_SPACE_CAP`] is 768 MiB on a 32-bit target, small enough to bind
/// ahead of the pixel and memory ceilings every caller here is actually about.
/// Passing `u64::MAX` keeps those subjects separate;
/// [`address_cap_binds_a_32_bit_process`] covers the real value on the targets
/// where it is not the maximum.
fn budget_with(memory: u64, concurrency: usize) -> SystemBudget {
    budget_with_cap(memory, concurrency, u64::MAX)
}

fn budget_with_cap(memory: u64, concurrency: usize, address_cap: u64) -> SystemBudget {
    SystemBudget {
        available_memory: memory,
        address_cap,
        concurrency,
    }
}

/// Build a `LimitSet` for the `check` tests, whose subject is the pixel
/// ceilings. The byte ceiling is left unbounded so it cannot accidentally
/// become the thing under test.
fn pixel_limits(max_width: u64, max_height: u64, max_pixels: u64, binding: Binding) -> LimitSet {
    LimitSet {
        max_width,
        max_height,
        max_pixels,
        max_bytes: u64::MAX,
        binding,
        bytes_binding: Binding::Memory,
    }
}

#[test]
fn webp_cap_matches_libwebp_constant() {
    let caps = format_caps(ImageFormatId::WebP);
    assert_eq!(caps.max_side, 16383);
}

#[test]
fn jpeg_cap_matches_the_decoder_ceiling() {
    // Not libjpeg's 65500: the zune decoder refuses anything above its own
    // `max_width` default before the codec runs, and a pre-check that allowed
    // more would pass files that then fail to decode.
    let caps = format_caps(ImageFormatId::Jpeg);
    assert_eq!(caps.max_side, DECODER_SIDE_LIMIT);
    assert_eq!(caps.max_side, 16384);
}

#[test]
fn png_cap_matches_the_decoder_ceiling() {
    // PNG publishes no side limit of its own, but the decoder applies the same
    // default ceiling as JPEG, so the cap has to reflect that rather than
    // leaving the format unbounded.
    let caps = format_caps(ImageFormatId::Png);
    assert_eq!(caps.max_side, DECODER_SIDE_LIMIT);
}

#[test]
fn avif_cap_matches_spec() {
    let caps = format_caps(ImageFormatId::Avif);
    assert_eq!(caps.max_side, 65536);
}

#[test]
fn formats_without_published_limits_are_unbounded() {
    for format in [
        ImageFormatId::Tiff,
        ImageFormatId::Svg,
        ImageFormatId::Other,
    ] {
        let caps = format_caps(format);
        assert_eq!(
            caps.max_side,
            u64::MAX,
            "{} must not claim a published dimension limit",
            format.name()
        );
        assert_eq!(caps.max_pixels, u64::MAX);
    }
}

#[test]
fn unprobed_machine_falls_back_instead_of_failing() {
    let budget = budget_with(0, 1);
    assert!(!budget.is_probed());
    assert_eq!(budget.per_image_bytes(), FALLBACK_PER_IMAGE_BYTES);
}

#[test]
fn budget_is_halved_and_divided_by_concurrency() {
    let budget = budget_with(8 * 1024 * 1024 * 1024, 4);
    // Half of 8 GiB is 4 GiB, split across 4 concurrent images.
    assert_eq!(budget.per_image_bytes(), 1024 * 1024 * 1024);
}

/// Plenty of free memory does not mean one large allocation is obtainable: a
/// 32-bit process needs a single contiguous range, and fragmentation is what
/// turns that into a failed allocation rather than a real OOM. The cap has to
/// bind before the halved budget does.
#[cfg(target_pointer_width = "32")]
#[test]
fn address_cap_binds_a_32_bit_process() {
    // 4 GiB usable after the safety halving, well above the cap.
    let budget = budget_with_cap(8 * 1024 * 1024 * 1024, 1, ADDRESS_SPACE_CAP);

    assert_eq!(budget.per_image_bytes(), ADDRESS_SPACE_CAP);
}

#[test]
fn concurrency_of_zero_is_treated_as_one() {
    let budget = budget_with(1024 * 1024 * 1024, 0);
    assert_eq!(budget.per_image_bytes(), 512 * 1024 * 1024);
}

#[test]
fn bytes_per_pixel_reflects_depth_and_colorspace() {
    assert_eq!(bytes_per_pixel(BitDepth::Eight, ColorSpace::RGB), 3);
    assert_eq!(bytes_per_pixel(BitDepth::Eight, ColorSpace::RGBA), 4);
    assert_eq!(bytes_per_pixel(BitDepth::Sixteen, ColorSpace::RGBA), 8);
    assert_eq!(bytes_per_pixel(BitDepth::Float32, ColorSpace::RGBA), 16);
    assert_eq!(bytes_per_pixel(BitDepth::Eight, ColorSpace::Luma), 1);
    // An unknown colour space must still produce a usable divisor.
    assert_eq!(bytes_per_pixel(BitDepth::Unknown, ColorSpace::Unknown), 1);
}

#[test]
fn memory_budget_binds_before_format_when_memory_is_tight() {
    // 1 GiB total, so 512 MiB usable; RGBA8 with a 3+16 cost is 76 bytes/px.
    let budget = budget_with(1024 * 1024 * 1024, 1);
    let limits = LimitSet::for_input(
        ImageFormatId::Avif,
        BitDepth::Eight,
        ColorSpace::RGBA,
        &budget,
        PipelineCost::for_encoder(ImageFormatId::Avif),
    );

    assert_eq!(limits.binding, Binding::Memory);
    assert!(limits.max_pixels < 65536 * 65536);
}

#[test]
fn format_cap_binds_when_memory_is_plentiful() {
    // 1 TiB available; the WebP side limit must win.
    let budget = budget_with(1024u64 * 1024 * 1024 * 1024, 1);
    let limits = LimitSet::for_input(
        ImageFormatId::WebP,
        BitDepth::Eight,
        ColorSpace::RGBA,
        &budget,
        PipelineCost::for_encoder(ImageFormatId::WebP),
    );

    assert_eq!(limits.max_width, 16383);
    assert_eq!(limits.max_height, 16383);
}

#[test]
fn check_reports_width_before_pixels() {
    let limits = pixel_limits(100, 200, 10_000, Binding::Format);

    // The area passes, so only the side check can reject this input.
    let violation = limits.check(101, 10).unwrap_err();
    assert_eq!(violation.kind, ViolationKind::Width);
    assert_eq!(violation.actual, 101);
    assert_eq!(violation.allowed, 100);
}

#[test]
fn check_reports_pixels_before_a_side() {
    let limits = pixel_limits(100, 100, 1_000, Binding::Memory);

    // Both a side and the area are too large. The area is reported because it
    // is the tighter description, and it carries the binding that produced it.
    let violation = limits.check(200, 200).unwrap_err();
    assert_eq!(violation.kind, ViolationKind::Pixels);
    assert_eq!(violation.binding, Binding::Memory);
}

#[test]
fn check_reports_height() {
    let limits = pixel_limits(100, 200, 10_000, Binding::Format);

    let violation = limits.check(10, 201).unwrap_err();
    assert_eq!(violation.kind, ViolationKind::Height);
}

#[test]
fn check_reports_pixel_product_when_both_sides_pass() {
    let limits = pixel_limits(1000, 1000, 10_000, Binding::Memory);

    // Each side is legal; only the product exceeds the ceiling.
    let violation = limits.check(500, 500).unwrap_err();
    assert_eq!(violation.kind, ViolationKind::Pixels);
    assert_eq!(violation.actual, 250_000);
    assert_eq!(violation.binding, Binding::Memory);

    assert!(limits.check(100, 100).is_ok());
}

#[test]
fn check_accepts_the_exact_limit() {
    let limits = pixel_limits(500, 500, 250_000, Binding::Format);

    assert!(limits.check(500, 500).is_ok());
}

#[test]
fn suggested_side_fits_under_every_ceiling() {
    let limits = pixel_limits(16383, 16383, 10_000_000, Binding::Memory);

    let side = limits.suggested_side();
    assert!(side <= limits.max_width);
    assert!(side.saturating_mul(side) <= limits.max_pixels);
    assert_eq!(side, 3162);
}

#[test]
fn suggested_side_is_at_least_one() {
    // Every ceiling is zero, including the byte one: the suggestion still has
    // to be a legal size rather than zero.
    let limits = LimitSet {
        max_width: 0,
        max_height: 0,
        max_pixels: 0,
        max_bytes: 0,
        binding: Binding::Memory,
        bytes_binding: Binding::Memory,
    };

    assert_eq!(limits.suggested_side(), 1);
}

#[test]
fn pipeline_cost_sums_all_stages() {
    let cost = PipelineCost::new(3, 2, 2, 16);
    assert_eq!(cost.total(), 23);
}

/// The encode half of a conversion belongs to the format being written.
///
/// Taking it from the input instead is what screened a JPEG-to-AVIF conversion
/// against JPEG's 6 buffers while the AVIF encoder went on to hold 19: the
/// encode buffers dominate the total, and only the output format knows them.
#[test]
fn conversion_cost_takes_its_encode_half_from_the_output_format() {
    let jpeg_to_jpeg = PipelineCost::for_conversion(ImageFormatId::Jpeg, ImageFormatId::Jpeg);
    let jpeg_to_avif = PipelineCost::for_conversion(ImageFormatId::Jpeg, ImageFormatId::Avif);

    assert_eq!(
        jpeg_to_avif.encode,
        PipelineCost::for_encoder(ImageFormatId::Avif).encode
    );
    assert_eq!(
        jpeg_to_avif.decode,
        PipelineCost::for_encoder(ImageFormatId::Jpeg).decode
    );
    assert!(
        jpeg_to_avif.total() > jpeg_to_jpeg.total(),
        "encoding to avif must cost more than encoding to jpeg: \
         {jpeg_to_avif:?} vs {jpeg_to_jpeg:?}"
    );
}

/// Swapping the arguments must change the answer, which is the whole point of
/// taking two: a PNG read for a JPEG output is not a JPEG read for a PNG
/// output, and collapsing them would silently reintroduce the wrong estimate.
#[test]
fn conversion_cost_is_not_symmetric() {
    let png_to_jpeg = PipelineCost::for_conversion(ImageFormatId::Png, ImageFormatId::Jpeg);
    let jpeg_to_png = PipelineCost::for_conversion(ImageFormatId::Jpeg, ImageFormatId::Png);

    assert!(png_to_jpeg.total() < jpeg_to_png.total());
}

/// Every assumed layout has to be at least as wide as 8-bit RGB, or a plain
/// photograph would be sized as if it were narrower than it is and the screen
/// would admit images it cannot hold.
#[test]
fn assumed_layouts_are_never_narrower_than_rgb8() {
    for format in [
        ImageFormatId::Jpeg,
        ImageFormatId::Png,
        ImageFormatId::WebP,
        ImageFormatId::Avif,
        ImageFormatId::Tiff,
        ImageFormatId::Svg,
        ImageFormatId::Other,
    ] {
        let (depth, colorspace) = format.max_pixel_layout();
        let assumed = bytes_per_pixel(depth, colorspace);
        let rgb8 = bytes_per_pixel(BitDepth::Eight, ColorSpace::RGB);

        assert!(
            assumed >= rgb8,
            "{format:?} assumes {assumed} B/px, below the {rgb8} B/px floor"
        );
    }
}

/// The screen has to assume the widest layout the format allows, because it
/// runs before the image exists. A 16-bit RGBA PNG is the case an RGB8 floor
/// would have sized four times too small.
#[test]
fn the_assumed_layout_covers_the_widest_the_format_allows() {
    let (depth, colorspace) = ImageFormatId::Png.max_pixel_layout();
    assert_eq!(bytes_per_pixel(depth, colorspace), 8);

    let (depth, colorspace) = ImageFormatId::Jpeg.max_pixel_layout();
    assert_eq!(bytes_per_pixel(depth, colorspace), 4);
}

#[test]
fn format_extension_lookup_is_case_insensitive() {
    assert_eq!(ImageFormatId::from_extension("JPG"), ImageFormatId::Jpeg);
    assert_eq!(ImageFormatId::from_extension("jpeg"), ImageFormatId::Jpeg);
    assert_eq!(ImageFormatId::from_extension("WebP"), ImageFormatId::WebP);
    assert_eq!(ImageFormatId::from_extension("tif"), ImageFormatId::Tiff);
    assert_eq!(ImageFormatId::from_extension("svgz"), ImageFormatId::Svg);
    assert_eq!(ImageFormatId::from_extension("qoi"), ImageFormatId::Other);
}

#[test]
fn binding_descriptions_are_not_empty() {
    for binding in [
        Binding::Format,
        Binding::Memory,
        Binding::Disk,
        Binding::None,
    ] {
        assert!(!binding.describe().is_empty());
    }
}

/// The free-space probe must answer for the case it exists for: an output file
/// that does not exist yet, in a directory that does. A bare `canonicalize` on
/// the file itself fails here, which used to silently disable the check.
#[cfg(feature = "limits")]
#[test]
fn free_space_is_found_for_a_not_yet_created_output() {
    let missing_output = std::env::temp_dir().join(format!(
        "rimage-limits-{}-not-created-yet/out.png",
        std::process::id()
    ));

    let free = free_space_at(&missing_output);
    assert!(
        free.is_some(),
        "free space under the temp dir must be determinable"
    );
    assert!(free.unwrap() > 0);
}

/// Even when several trailing components are missing, the probe walks up to
/// the nearest existing ancestor instead of giving up.
#[cfg(feature = "limits")]
#[test]
fn free_space_walks_up_past_missing_directories() {
    let deep = std::env::temp_dir().join(format!(
        "rimage-limits-{}/missing/deeper/out.png",
        std::process::id()
    ));

    assert!(free_space_at(&deep).is_some());
}

/// An image beyond the decoder's ceiling must be rejected before any decoding
/// is attempted, on every realistic memory budget.
///
/// The ceiling is the zune decoder's own 16384, so a 65500x2466 image — which
/// the underlying codecs could express — is rejected on its *width*. That is
/// the intended behaviour: the decoder refuses it from the header anyway, and
/// catching it here is what makes the message name a limit instead of quoting
/// the decoder's internal one.
#[test]
fn dimensions_beyond_the_decoder_ceiling_are_rejected() {
    // The largest fixture in the corpus, and the size the task calls extreme.
    const EXTREME_WIDTH: u64 = 65500;
    const EXTREME_HEIGHT: u64 = 2466;

    // Anything at or above this produces the same budget, so this figure means
    // "more memory than any process can address on 64-bit".
    const PLENTIFUL: u64 = u64::MAX / 4 + 1;
    const TYPICAL: u64 = 64 * 1024 * 1024 * 1024;

    let limits = |memory: u64| {
        let budget = budget_with(memory, 1);
        LimitSet::for_input(
            ImageFormatId::Jpeg,
            BitDepth::Eight,
            ColorSpace::RGB,
            &budget,
            PipelineCost::for_encoder(ImageFormatId::Jpeg),
        )
    };

    // The ceiling is the decoder's, not libjpeg's larger figure.
    assert_eq!(limits(PLENTIFUL).max_width, 16384);
    assert!(limits(PLENTIFUL).check(16384, 1).is_ok());

    // The extreme panorama is rejected on its width, even with memory to spare.
    let violation = limits(PLENTIFUL)
        .check(EXTREME_WIDTH, EXTREME_HEIGHT)
        .unwrap_err();
    assert_eq!(violation.kind, ViolationKind::Width);
    assert_eq!(violation.binding, Binding::Format);

    // Memory is the binding constraint for large images that are within the
    // side ceiling. `max_pixels` is derived from the budget rather than
    // restated here, so this compares the two ceilings instead of re-deriving
    // one of them by hand.
    const BIG_SIDE: u64 = 16384;
    // Small enough that the memory budget is tighter than the side ceiling,
    // which is the state this branch is meant to exercise.
    const CONSTRAINED: u64 = 4 * 1024 * 1024 * 1024;
    let bound_by_memory = limits(CONSTRAINED);
    assert!(
        bound_by_memory.max_pixels < BIG_SIDE * BIG_SIDE,
        "a {CONSTRAINED}-byte budget admitted {} pixels, which is not below the \
         {}-pixel square at the side ceiling",
        bound_by_memory.max_pixels,
        BIG_SIDE * BIG_SIDE
    );
    assert_eq!(bound_by_memory.binding, Binding::Memory);

    // At the side ceiling with room to spare, the format cap is what binds.
    assert_eq!(limits(PLENTIFUL).binding, Binding::Format);
    assert_eq!(limits(PLENTIFUL).max_pixels, BIG_SIDE * BIG_SIDE);

    // A square that both the format and the memory budget can express is
    // accepted, so the checks do not reject everything indiscriminately.
    assert!(limits(TYPICAL).check(8_000, 8_000).is_ok());
}

#[test]
fn png_is_bounded_by_the_same_decoder_ceiling() {
    let budget = budget_with(u64::MAX / 4 + 1, 1);
    let limits = LimitSet::for_input(
        ImageFormatId::Png,
        BitDepth::Eight,
        ColorSpace::RGBA,
        &budget,
        PipelineCost::for_encoder(ImageFormatId::Png),
    );

    assert_eq!(limits.max_width, DECODER_SIDE_LIMIT);
    assert_eq!(limits.max_height, DECODER_SIDE_LIMIT);
    assert!(limits.check(16384, 16384).is_ok());
    assert_eq!(
        limits.check(16385, 1).unwrap_err().kind,
        ViolationKind::Width
    );
}

#[test]
fn encoder_names_map_to_the_format_they_write() {
    // The CLI subcommand is not the file extension: `mozjpeg` writes a `.jpg`
    // and `oxipng` writes a `.png`. Looking output limits up by subcommand is
    // what makes them describe the file that actually lands on disk.
    assert_eq!(
        ImageFormatId::from_encoder_name("mozjpeg"),
        ImageFormatId::Jpeg
    );
    assert_eq!(
        ImageFormatId::from_encoder_name("jpeg"),
        ImageFormatId::Jpeg
    );
    assert_eq!(
        ImageFormatId::from_encoder_name("oxipng"),
        ImageFormatId::Png
    );
    assert_eq!(ImageFormatId::from_encoder_name("png"), ImageFormatId::Png);
    assert_eq!(
        ImageFormatId::from_encoder_name("webp"),
        ImageFormatId::WebP
    );
    assert_eq!(
        ImageFormatId::from_encoder_name("avif"),
        ImageFormatId::Avif
    );
    assert_eq!(
        ImageFormatId::from_encoder_name("tiff"),
        ImageFormatId::Tiff
    );
    assert_eq!(
        ImageFormatId::from_encoder_name("qoi"),
        ImageFormatId::Other
    );
}

#[test]
fn check_bytes_reports_the_byte_binding() {
    let limits = LimitSet {
        max_width: u64::MAX,
        max_height: u64::MAX,
        max_pixels: u64::MAX,
        max_bytes: 1024,
        binding: Binding::Format,
        bytes_binding: Binding::Disk,
    };

    let violation = limits.check_bytes(2048).unwrap_err();
    assert_eq!(violation.kind, ViolationKind::Bytes);
    assert_eq!(violation.actual, 2048);
    assert_eq!(violation.allowed, 1024);

    // The byte ceiling reports *its own* binding, not the pixel one. Mixing
    // them made a full volume report itself as a format limit.
    assert_eq!(violation.binding, Binding::Disk);

    assert!(limits.check_bytes(1024).is_ok());
}

#[test]
fn byte_and_pixel_bindings_are_tracked_separately() {
    // A limit set whose pixels are format-bound but whose bytes are disk-bound
    // must answer each question with the right source.
    let limits = LimitSet {
        max_width: 100,
        max_height: 100,
        max_pixels: 10_000,
        max_bytes: 4096,
        binding: Binding::Format,
        bytes_binding: Binding::Disk,
    };

    assert_eq!(limits.check(101, 1).unwrap_err().binding, Binding::Format);
    assert_eq!(limits.check_bytes(8192).unwrap_err().binding, Binding::Disk);
}
