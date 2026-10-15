//! Accès au matériel de la Miyoo Mini : volume, rétroéclairage, écran,
//! batterie et arrêt. Les chemins et formules sont ceux de Telmi OS
//! (`volume.h`, `display.h`, `battery.h`, `system.h`).
//!
//! Hors de la console (simulateur), tout est sans effet.

use std::path::Path;

use crate::config::Paths;

pub const VOLUME_MAX: i32 = 25;
pub const BRIGHTNESS_MAX: i32 = 10;

pub struct Hardware {
    enabled: bool,
    screen_on: bool,
    volume: i32,
    brightness: i32,
}

impl Hardware {
    pub fn detect() -> Hardware {
        let enabled = cfg!(target_arch = "arm") && Path::new("/dev/mi_ao").exists();
        Hardware { enabled, screen_on: true, volume: 0, brightness: 0 }
    }

    pub fn screen_on(&self) -> bool {
        self.screen_on
    }

    pub fn volume(&self) -> i32 {
        self.volume
    }

    pub fn brightness(&self) -> i32 {
        self.brightness
    }

    /// Volume de 0 à 25 (au-delà de 20, amplification), courbe de Telmi.
    pub fn set_volume(&mut self, volume: i32) {
        self.volume = volume.clamp(0, VOLUME_MAX);
        if !self.enabled {
            return;
        }
        let base = self.volume.min(20);
        let boost = (self.volume - 20).max(0);
        let raw = if base == 0 { 0 } else { (48.0 * (1.0 + base as f64).log10()).round() as i32 + boost };
        set_volume_raw(raw);
    }

    /// Luminosité de 0 à 10, courbe exponentielle de Telmi.
    pub fn set_brightness(&mut self, brightness: i32) {
        self.brightness = brightness.clamp(0, BRIGHTNESS_MAX);
        if !self.enabled {
            return;
        }
        let raw = (3.0 * (0.350656 * self.brightness as f64).exp()).round() as u32;
        write_sync(&format!("{PWM_DIR}pwm0/duty_cycle"), &raw.to_string());
    }

    /// Allume ou éteint l'écran (rétroéclairage par GPIO 4).
    pub fn set_screen(&mut self, on: bool) {
        if self.screen_on == on {
            return;
        }
        self.screen_on = on;
        if !self.enabled {
            return;
        }
        write("/sys/class/gpio/export", "4");
        write("/sys/devices/gpiochip0/gpio/gpio4/direction", "out");
        write("/sys/devices/gpiochip0/gpio/gpio4/value", if on { "1" } else { "0" });
        write("/sys/class/gpio/unexport", "4");
        if on {
            write(&format!("{PWM_DIR}export"), "0");
            write(&format!("{PWM_DIR}pwm0/enable"), "0");
            write(&format!("{PWM_DIR}pwm0/enable"), "1");
        }
    }

    /// Pourcentage écrit par `batmon` dans `/tmp/percBat`.
    pub fn battery_percent(&self) -> i32 {
        if !self.enabled {
            return 100;
        }
        std::fs::read_to_string("/tmp/percBat")
            .ok()
            .and_then(|s| s.trim().parse::<i32>().ok())
            .map_or(0, |p| p.clamp(0, 100))
    }

    /// Demande l'arrêt à `runtime.sh` et sauvegarde l'heure, comme `system_shutdown`.
    pub fn request_shutdown(&self, paths: &Paths) {
        if !self.enabled {
            return;
        }
        let _ = std::fs::write("/tmp/.offOrder", "1");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let _ = telmi_story::save::write_atomic(&paths.clock_file(), now.to_string().as_bytes());
        // SAFETY: sync() n'a pas de précondition.
        unsafe { libc::sync() };
    }
}

const PWM_DIR: &str = "/sys/devices/soc0/soc/1f003400.pwm/pwm/pwmchip0/";

fn write(path: &str, value: &str) {
    let _ = std::fs::write(path, value);
}

fn write_sync(path: &str, value: &str) {
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new().write(true).truncate(true).open(path) {
        let _ = file.write_all(value.as_bytes());
        let _ = file.sync_all();
    }
}

/// Volume brut par ioctl sur `/dev/mi_ao`, entre -60 (muet) et +30.
#[cfg(target_arch = "arm")]
fn set_volume_raw(value: i32) {
    const MIN_RAW: i32 = -60;
    const MAX_RAW: i32 = 30;
    const MI_AO_SETVOLUME: libc::c_ulong = 0x4008_690b;
    const MI_AO_GETVOLUME: libc::c_ulong = 0xc008_690c;
    const MI_AO_SETMUTE: libc::c_ulong = 0x4008_690d;

    use std::os::fd::AsRawFd;
    let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/mi_ao") else {
        return;
    };
    let fd = file.as_raw_fd();
    let mut args: [i32; 2] = [0, 0];
    // Le pilote lit et écrit `args` à travers ce pointeur : on n'y accède plus autrement.
    let arg = args.as_mut_ptr();
    let mut request: [u64; 2] = [std::mem::size_of_val(&args) as u64, arg as usize as u64];
    let request = request.as_mut_ptr();
    // SAFETY: structure attendue par le pilote mi_ao (taille, pointeur), comme dans
    // Telmi ; `args` vit jusqu'à la fin de la fonction.
    unsafe {
        libc::ioctl(fd, MI_AO_GETVOLUME, request);
        let previous = arg.add(1).read_volatile();
        let value = (value + MIN_RAW).clamp(MIN_RAW, MAX_RAW);
        if value == previous {
            return;
        }
        arg.add(1).write_volatile(value);
        libc::ioctl(fd, MI_AO_SETVOLUME, request);
        let mute = if previous <= MIN_RAW && value > MIN_RAW {
            Some(0)
        } else if previous > MIN_RAW && value <= MIN_RAW {
            Some(1)
        } else {
            None
        };
        if let Some(mute) = mute {
            arg.add(1).write_volatile(mute);
            libc::ioctl(fd, MI_AO_SETMUTE, request);
        }
    }
}

#[cfg(not(target_arch = "arm"))]
fn set_volume_raw(_value: i32) {}
