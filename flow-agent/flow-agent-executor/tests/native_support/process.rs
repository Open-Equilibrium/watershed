use std::{
    io,
    process::{Child, ExitStatus},
    thread,
    time::{Duration, Instant},
};

pub fn wait_for_exit(child: &mut Child) -> io::Result<ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "test Executor did not exit",
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }
}
