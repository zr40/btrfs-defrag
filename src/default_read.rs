use std::io::{Read, Result};

/// Wraps a `Read`, so that `std::io::copy` doesn't specialize.
///
/// When `std::io::copy` is used to copy between `File`s, its specialization uses `copy_file_range`.
/// This copies the file contents using reflinks, which does not accomplish our goal of defragmenting the file.
///
/// `DefaultRead` avoids this specialization.
pub(crate) struct DefaultRead<T: Read>(pub T);

impl<T: Read> Read for DefaultRead<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.0.read(buf)
    }
}
