use crate::recovery::write_atomic;
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager, Runtime, State};
use zeroize::Zeroizing;

pub const LOCKED: &str = "应用已锁定，请先输入密码解锁";
const CORRUPTED: &str = "应用锁配置损坏。请检查或移除应用数据目录中的 app-lock.json，再重新启动。";
const NOT_ENABLED: &str = "尚未开启应用锁";
const UNAVAILABLE: &str = "应用锁暂不可用";
const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const MEMORY_COST: u32 = 19_456;
const TIME_COST: u32 = 2;
const PARALLELISM: u32 = 1;
const SALT_BYTES: usize = 16;
const HASH_BYTES: usize = 32;

#[derive(Clone, Deserialize, Serialize)]
struct Config {
    version: u32,
    password_hash: String,
    recovery_hash: String,
    idle_timeout_minutes: u32,
    updated_at: String,
}

enum Configuration {
    Disabled,
    Enabled(Config),
    Corrupted,
}

pub struct AppLock {
    path: PathBuf,
    state: Mutex<LockState>,
}

struct LockState {
    config: Configuration,
    locked: bool,
    failures: u8,
    retry_at: Option<Instant>,
}

#[derive(Serialize)]
pub struct AppLockStatus {
    enabled: bool,
    locked: bool,
    corrupted: bool,
    idle_timeout_minutes: u32,
    retry_after_seconds: u64,
    updated_at: Option<String>,
}

#[derive(Serialize)]
pub struct AppLockWithRecoveryCode {
    status: AppLockStatus,
    recovery_code: String,
}

fn valid_idle_timeout(minutes: u32) -> Result<u32, String> {
    if matches!(minutes, 0 | 5 | 10 | 15 | 30) {
        Ok(minutes)
    } else {
        Err("无效的空闲自动锁定时长".into())
    }
}

fn valid_password(password: &str) -> bool {
    (6..=64).contains(&password.chars().take(65).count())
        && !password.chars().any(char::is_control)
        && !password.chars().all(char::is_whitespace)
}

fn check_password(password: &str) -> Result<(), String> {
    valid_password(password)
        .then_some(())
        .ok_or_else(|| "密码须为 6 至 64 个字符，且不能全为空白或含控制字符。".into())
}

fn hash_secret(secret: &str) -> Result<String, String> {
    let mut salt = [0; SALT_BYTES];
    getrandom::fill(&mut salt).map_err(|_| UNAVAILABLE)?;
    let salt = SaltString::encode_b64(&salt).map_err(|_| UNAVAILABLE)?;
    let params = Params::new(MEMORY_COST, TIME_COST, PARALLELISM, Some(HASH_BYTES))
        .map_err(|_| UNAVAILABLE)?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(secret.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| UNAVAILABLE.into())
}

fn parse_hash(value: &str) -> Option<PasswordHash<'_>> {
    let hash = PasswordHash::new(value).ok()?;
    let mut decoded_salt = [0; 64];
    if hash.algorithm.as_str() != "argon2id"
        || hash.version != Some(19)
        || hash.params.iter().count() != 3
        || hash.params.get_decimal("m") != Some(MEMORY_COST)
        || hash.params.get_decimal("t") != Some(TIME_COST)
        || hash.params.get_decimal("p") != Some(PARALLELISM)
        || hash.salt?.decode_b64(&mut decoded_salt).ok()?.len() != SALT_BYTES
        || hash.hash?.len() != HASH_BYTES
    {
        return None;
    }
    Some(hash)
}

fn generate_recovery_code() -> Result<Zeroizing<String>, String> {
    // The alphabet has 32 symbols, so each random byte maps without bias.
    let mut bytes = Zeroizing::new([0; 16]);
    getrandom::fill(bytes.as_mut()).map_err(|_| UNAVAILABLE)?;
    Ok(Zeroizing::new(
        bytes
            .iter()
            .map(|byte| char::from(ALPHABET[usize::from(*byte) % ALPHABET.len()]))
            .collect(),
    ))
}

fn normalize_recovery_code(code: &str) -> Zeroizing<String> {
    Zeroizing::new(
        code.chars()
            .filter(|ch| !ch.is_whitespace() && *ch != '-')
            .take(17)
            .map(|ch| ch.to_ascii_uppercase())
            .collect(),
    )
}

fn format_recovery_code(code: &str) -> String {
    format!(
        "{}-{}-{}-{}",
        &code[..4],
        &code[4..8],
        &code[8..12],
        &code[12..]
    )
}

fn load_config(path: &Path) -> Configuration {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Configuration::Disabled,
        Ok(metadata) if metadata.is_file() && metadata.len() <= 8192 => {}
        _ => return Configuration::Corrupted,
    }
    let config = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Config>(&bytes).ok())
        .filter(|config| {
            config.version == 1
                && valid_idle_timeout(config.idle_timeout_minutes).is_ok()
                && parse_hash(&config.password_hash).is_some()
                && parse_hash(&config.recovery_hash).is_some()
        });
    config.map_or(Configuration::Corrupted, Configuration::Enabled)
}

impl AppLock {
    #[cfg(test)]
    pub fn disabled() -> Self {
        Self {
            path: PathBuf::new(),
            state: Mutex::new(LockState {
                config: Configuration::Disabled,
                locked: false,
                failures: 0,
                retry_at: None,
            }),
        }
    }

    pub fn load(directory: PathBuf) -> Self {
        let path = directory.join("app-lock.json");
        let config = load_config(&path);
        let locked = !matches!(config, Configuration::Disabled);
        Self {
            path,
            state: Mutex::new(LockState {
                config,
                locked,
                failures: 0,
                retry_at: None,
            }),
        }
    }

    fn state(&self) -> Result<MutexGuard<'_, LockState>, String> {
        self.state.lock().map_err(|_| UNAVAILABLE.into())
    }

    fn save(&self, state: &mut LockState, mut config: Config) -> Result<(), String> {
        config.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| UNAVAILABLE)?
            .as_secs()
            .to_string();
        let bytes = serde_json::to_vec_pretty(&config).map_err(|_| UNAVAILABLE)?;
        write_atomic(&self.path, &bytes).map_err(|_| "保存应用锁配置失败")?;
        state.config = Configuration::Enabled(config);
        Ok(())
    }
}

impl LockState {
    fn config(&self) -> Result<&Config, String> {
        match &self.config {
            Configuration::Disabled => Err(NOT_ENABLED.into()),
            Configuration::Corrupted => Err(CORRUPTED.into()),
            Configuration::Enabled(config) => Ok(config),
        }
    }

    fn ensure_unlocked(&self) -> Result<(), String> {
        if self.locked {
            Err(LOCKED.into())
        } else {
            Ok(())
        }
    }

    fn retry_after_seconds(&self) -> u64 {
        self.retry_at
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .map_or(0, |duration| {
                duration.as_secs() + u64::from(duration.subsec_nanos() > 0)
            })
    }

    fn status(&self) -> AppLockStatus {
        let config = self.config().ok();
        AppLockStatus {
            enabled: !matches!(self.config, Configuration::Disabled),
            locked: self.locked,
            corrupted: matches!(self.config, Configuration::Corrupted),
            idle_timeout_minutes: config.map_or(0, |config| config.idle_timeout_minutes),
            retry_after_seconds: self.retry_after_seconds(),
            updated_at: config.map(|config| config.updated_at.clone()),
        }
    }

    fn verify(&mut self, secret: &str, recovery: bool) -> Result<(), String> {
        let config = self.config()?;
        let retry = self.retry_after_seconds();
        if retry > 0 {
            return Err(cooldown_message(retry));
        }
        let (hash, valid, error) = if recovery {
            (
                &config.recovery_hash,
                secret.len() == 16 && secret.bytes().all(|c| ALPHABET.contains(&c)),
                "恢复码不正确",
            )
        } else {
            (&config.password_hash, valid_password(secret), "密码不正确")
        };
        let parsed = parse_hash(hash).ok_or(CORRUPTED)?;
        if valid
            && Argon2::default()
                .verify_password(secret.as_bytes(), &parsed)
                .is_ok()
        {
            self.failures = 0;
            self.retry_at = None;
            return Ok(());
        }
        self.failures = self.failures.saturating_add(1);
        let seconds = match self.failures {
            0..=4 => return Err(error.into()),
            5 => 5,
            6 => 15,
            7 => 30,
            _ => 60,
        };
        self.retry_at = Some(Instant::now() + Duration::from_secs(seconds));
        Err(cooldown_message(seconds))
    }
}

fn cooldown_message(seconds: u64) -> String {
    format!("密码错误次数过多，请在 {seconds} 秒后重试")
}

pub fn ensure_unlocked<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    with_unlocked(app, || Ok(()))
}

pub fn with_unlocked<R: Runtime, T>(
    app: &AppHandle<R>,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    // Hold the state through short commits so locking cannot interleave with the final rename.
    let lock = app.try_state::<AppLock>().ok_or(UNAVAILABLE)?;
    let state = lock.state()?;
    state.ensure_unlocked()?;
    operation()
}

#[tauri::command]
pub fn app_lock_status(lock: State<'_, AppLock>) -> Result<AppLockStatus, String> {
    Ok(lock.state()?.status())
}

#[tauri::command(async)]
pub fn unlock_app(lock: State<'_, AppLock>, password: String) -> Result<AppLockStatus, String> {
    let password = Zeroizing::new(password);
    let mut state = lock.state()?;
    state.verify(&password, false)?;
    state.locked = false;
    Ok(state.status())
}

#[tauri::command(async)]
pub fn recover_app_lock(
    lock: State<'_, AppLock>,
    recovery_code: String,
    new_password: String,
) -> Result<AppLockWithRecoveryCode, String> {
    let recovery_code = Zeroizing::new(recovery_code);
    let new_password = Zeroizing::new(new_password);
    let mut state = lock.state()?;
    let mut config = state.config()?.clone();
    check_password(&new_password)?;
    state.verify(&normalize_recovery_code(&recovery_code), true)?;
    let new_code = generate_recovery_code()?;
    config.password_hash = hash_secret(&new_password)?;
    config.recovery_hash = hash_secret(&new_code)?;
    lock.save(&mut state, config)?;
    state.locked = false;
    Ok(AppLockWithRecoveryCode {
        status: state.status(),
        recovery_code: format_recovery_code(&new_code),
    })
}

#[tauri::command]
pub fn lock_app(lock: State<'_, AppLock>) -> Result<AppLockStatus, String> {
    let mut state = lock.state()?;
    state.config()?;
    state.locked = true;
    Ok(state.status())
}

#[tauri::command(async)]
pub fn setup_app_lock(
    lock: State<'_, AppLock>,
    password: String,
    idle_timeout_minutes: Option<u32>,
) -> Result<AppLockWithRecoveryCode, String> {
    let password = Zeroizing::new(password);
    let mut state = lock.state()?;
    state.ensure_unlocked()?;
    if !matches!(state.config, Configuration::Disabled) {
        return Err("应用锁已开启".into());
    }
    check_password(&password)?;
    let idle_timeout_minutes = valid_idle_timeout(idle_timeout_minutes.unwrap_or(0))?;
    let code = generate_recovery_code()?;
    let config = Config {
        version: 1,
        password_hash: hash_secret(&password)?,
        recovery_hash: hash_secret(&code)?,
        idle_timeout_minutes,
        updated_at: String::new(),
    };
    lock.save(&mut state, config)?;
    Ok(AppLockWithRecoveryCode {
        status: state.status(),
        recovery_code: format_recovery_code(&code),
    })
}

#[tauri::command(async)]
pub fn change_app_lock_password(
    lock: State<'_, AppLock>,
    current_password: String,
    new_password: String,
) -> Result<AppLockStatus, String> {
    let current_password = Zeroizing::new(current_password);
    let new_password = Zeroizing::new(new_password);
    let mut state = lock.state()?;
    state.ensure_unlocked()?;
    check_password(&new_password)?;
    state.verify(&current_password, false)?;
    let mut config = state.config()?.clone();
    config.password_hash = hash_secret(&new_password)?;
    lock.save(&mut state, config)?;
    Ok(state.status())
}

#[tauri::command(async)]
pub fn regenerate_recovery_code(
    lock: State<'_, AppLock>,
    current_password: String,
) -> Result<AppLockWithRecoveryCode, String> {
    let current_password = Zeroizing::new(current_password);
    let mut state = lock.state()?;
    state.ensure_unlocked()?;
    state.verify(&current_password, false)?;
    let mut config = state.config()?.clone();
    let code = generate_recovery_code()?;
    config.recovery_hash = hash_secret(&code)?;
    lock.save(&mut state, config)?;
    Ok(AppLockWithRecoveryCode {
        status: state.status(),
        recovery_code: format_recovery_code(&code),
    })
}

#[tauri::command]
pub fn update_app_lock_settings(
    lock: State<'_, AppLock>,
    idle_timeout_minutes: u32,
) -> Result<AppLockStatus, String> {
    let mut state = lock.state()?;
    state.ensure_unlocked()?;
    let mut config = state.config()?.clone();
    config.idle_timeout_minutes = valid_idle_timeout(idle_timeout_minutes)?;
    lock.save(&mut state, config)?;
    Ok(state.status())
}

#[tauri::command(async)]
pub fn disable_app_lock(
    lock: State<'_, AppLock>,
    current_password: String,
) -> Result<AppLockStatus, String> {
    let current_password = Zeroizing::new(current_password);
    let mut state = lock.state()?;
    state.ensure_unlocked()?;
    state.verify(&current_password, false)?;
    match fs::remove_file(&lock.path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err("删除应用锁配置失败".into()),
    }
    state.config = Configuration::Disabled;
    Ok(state.status())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_shares_escalating_cooldown_and_resets_after_success() {
        let hash = hash_secret("valid-password").unwrap();
        let mut state = LockState {
            config: Configuration::Enabled(Config {
                version: 1,
                password_hash: hash.clone(),
                recovery_hash: hash,
                idle_timeout_minutes: 0,
                updated_at: String::new(),
            }),
            locked: true,
            failures: 0,
            retry_at: None,
        };
        for _ in 0..4 {
            assert_eq!(state.verify("wrong", false).unwrap_err(), "密码不正确");
        }
        for (index, expected) in [5, 15, 30, 60, 60].into_iter().enumerate() {
            assert_eq!(
                state.verify("bad", true).unwrap_err(),
                cooldown_message(expected)
            );
            assert_eq!(state.retry_after_seconds(), expected);
            assert_eq!(
                state.verify("valid-password", false).unwrap_err(),
                cooldown_message(expected)
            );
            assert_eq!(usize::from(state.failures), 5 + index);
            state.retry_at = Some(Instant::now());
        }
        state.verify("valid-password", false).unwrap();
        assert_eq!(state.failures, 0);
        assert_eq!(state.retry_after_seconds(), 0);
    }

    #[test]
    fn password_bounds_count_unicode_characters_and_preserve_whitespace() {
        assert!(!valid_password(&"座".repeat(5)));
        assert!(valid_password(&"座".repeat(6)));
        assert!(valid_password(&"座".repeat(64)));
        assert!(!valid_password(&"座".repeat(65)));
        assert!(!valid_password(
            "\u{3000}\u{3000}\u{3000}\u{3000}\u{3000}\u{3000}"
        ));
        assert!(!valid_password("班伴桌面应用\n"));
        assert!(valid_password(" 班伴桌面应用 "));
    }
}
