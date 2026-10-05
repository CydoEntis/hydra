//! Every process the daemon starts goes when the daemon goes, however it goes.
//!
//! On Windows each pane has a console host (OpenConsole / conhost) the daemon started. If the
//! daemon exits without closing it (killed, crashed, or just exiting quickly) the host keeps
//! running, hidden, often busy. Putting the daemon in a job object that kills its members
//! when its last handle closes ties them to the daemon's life: Windows closes the job when
//! the process ends, whatever the reason. That includes everything started from inside a
//! pane: an agent's tools start hidden consoles of their own, and letting those break away
//! left them running after the daemon. (A GUI app opened from a pane goes too.)
//! Elsewhere the panes' process groups get SIGHUP when the daemon's PTYs close.

/// Tie the daemon's child processes to its life. Safe to call more than once.
pub fn children_die_with_us() {
    #[cfg(windows)]
    {
        use std::sync::OnceLock;
        static JOB: OnceLock<usize> = OnceLock::new();
        JOB.get_or_init(|| match make_job() {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("couldn't tie panes to the daemon's life: {e}");
                0
            }
        });
    }
}

#[cfg(windows)]
fn make_job() -> Result<usize, String> {
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: plain Win32 calls with valid arguments; the job handle is kept open for the
    // rest of the process (closing it is what kills the members), so it's never freed.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if AssignProcessToJobObject(job, GetCurrentProcess()) == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(job as usize)
    }
}
