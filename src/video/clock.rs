use std::time::Instant;
/// A media clock for files without audio: wall time scaled by the playback
/// speed, which pauses and resumes.
pub struct Clock {
    base: f64,
    speed: f64,
    started: Option<Instant>,
}
impl Clock {
    pub fn new(base: f64, speed: f64) -> Self {
        Self {
            base,
            speed: speed.max(0.01),
            started: Some(Instant::now()),
        }
    }
    pub fn now(&self) -> f64 {
        match self.started {
            Some(t) => self.base + t.elapsed().as_secs_f64() * self.speed,
            None => self.base,
        }
    }
    pub fn pause(&mut self) {
        if self.started.is_some() {
            self.base = self.now();
            self.started = None;
        }
    }
    pub fn resume(&mut self) {
        if self.started.is_none() {
            self.started = Some(Instant::now());
        }
    }
    pub fn set_speed(&mut self, speed: f64) {
        let running = self.started.is_some();
        self.pause();
        self.speed = speed.max(0.01);
        if running {
            self.resume();
        }
    }
}
