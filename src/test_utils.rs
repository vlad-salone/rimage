use zune_core::{
    bit_depth::{BitDepth, BitType},
    colorspace::ColorSpace,
};
use zune_image::{channel::Channel, frame::Frame, image::Image};

pub(crate) fn create_test_image_u8(width: usize, height: usize, colorspace: ColorSpace) -> Image {
    Image::from_fn(width, height, colorspace, |x, y, px: &mut [u8; 4]| {
        let r = (0.3 * x as f32) as u8;
        let b = (0.3 * y as f32) as u8;

        px[0] = r;
        px[1] = 0;
        px[2] = b;
    })
}

pub(crate) fn create_test_image_u16(width: usize, height: usize, colorspace: ColorSpace) -> Image {
    Image::from_fn(width, height, colorspace, |x, y, px: &mut [u16; 4]| {
        let r = (0.3 * x as f32) as u16;
        let b = (0.3 * y as f32) as u16;

        px[0] = r;
        px[1] = 0;
        px[2] = b;
    })
}

pub(crate) fn create_test_image_f32(width: usize, height: usize, colorspace: ColorSpace) -> Image {
    Image::from_fn(width, height, colorspace, |x, y, px: &mut [f32; 4]| {
        let r = 0.3 * x as f32;
        let b = 0.3 * y as f32;

        px[0] = r;
        px[1] = 0.;
        px[2] = b;
    })
}

pub(crate) fn create_test_image_animated(
    width: usize,
    height: usize,
    colorspace: ColorSpace,
) -> Image {
    let mut frames = vec![];

    let channel_length = width * height;
    let num_components = colorspace.num_components();

    (0..=5).for_each(|_| {
        let channels =
            vec![Channel::new_with_bit_type(channel_length, BitType::U8); num_components];
        frames.push(Frame::new(channels))
    });

    Image::new_frames(frames, BitDepth::Eight, width, height, colorspace)
}
