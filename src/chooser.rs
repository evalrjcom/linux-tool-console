use fltk::dialog::FileDialogType;
use fltk_sys::dialog::{
    Fl_Native_File_Chooser, Fl_Native_File_Chooser_delete, Fl_Native_File_Chooser_errmsg,
    Fl_Native_File_Chooser_filename, Fl_Native_File_Chooser_new,
    Fl_Native_File_Chooser_set_directory, Fl_Native_File_Chooser_set_title,
    Fl_Native_File_Chooser_show,
};
use std::ffi::{CStr, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

// Own the chooser on the UI thread. The high-level FLTK wrapper decodes
// filenames lossily, which could select a different executable on Linux.
struct Chooser(NonNull<Fl_Native_File_Chooser>);

impl Drop for Chooser {
    fn drop(&mut self) {
        unsafe { Fl_Native_File_Chooser_delete(self.0.as_ptr()) };
    }
}

pub fn executable() -> io::Result<Option<PathBuf>> {
    // FLTK owns the returned C strings. Copy their bytes before deleting the
    // chooser, and check for null even when the backend reports success.
    unsafe {
        let dialog = Chooser(
            NonNull::new(Fl_Native_File_Chooser_new(
                FileDialogType::BrowseFile as i32,
            ))
            .ok_or_else(|| io::Error::other("Could not create a native file chooser"))?,
        );
        let handle = dialog.0.as_ptr();
        Fl_Native_File_Chooser_set_title(handle, c"Select an executable".as_ptr());
        if Path::new("/usr/bin").is_dir() {
            Fl_Native_File_Chooser_set_directory(handle, c"/usr/bin".as_ptr());
        }
        match Fl_Native_File_Chooser_show(handle) {
            0 => {
                let path = Fl_Native_File_Chooser_filename(handle);
                if path.is_null() {
                    Err(io::Error::other(
                        "The file chooser returned an empty selection",
                    ))
                } else {
                    Ok(Some(PathBuf::from(OsStr::from_bytes(
                        CStr::from_ptr(path).to_bytes(),
                    ))))
                }
            }
            1 => Ok(None),
            _ => {
                let error = Fl_Native_File_Chooser_errmsg(handle);
                let message = if error.is_null() {
                    "The native file chooser failed".into()
                } else {
                    CStr::from_ptr(error).to_string_lossy().into_owned()
                };
                Err(io::Error::other(message))
            }
        }
    }
}
