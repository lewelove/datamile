pub mod fast_rgb_to_bmp;

mod codec;
mod derivative;
mod error;
mod master;

#[cfg(test)]
mod tests;

pub use codec::{
    FilterType, FitMode, ImageWireFormat, MAX_IMAGE_DIMENSION,
    calculate_rendered_dimensions, decode_qoi_to_rgb, encode_rgb_image,
    filter_type_to_str, parse_filter, parse_fit_mode, parse_wire_format,
    read_qoi_dimensions, resize_image_fit, resize_image_max_edge, save_qoi_atomic,
};
pub use derivative::{
    derivative_filename, read_master_dimension, render_derivative_from_master,
};
pub use error::ImageError;
pub use master::{
    ImageSpec, RenderedMaster, compute_image_address, master_dir, master_path,
    render_master_from_file, write_master_qoi,
};
