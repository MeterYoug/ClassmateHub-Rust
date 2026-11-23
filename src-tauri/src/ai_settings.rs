use crate::recovery::write_atomic;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub endpoint: String,
    pub model: String,
    pub timeout_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

#[derive(Serialize)]
pub struct View {
    pub settings: Option<Settings>,
    pub has_key: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: u64,
    pub name: String,
    pub provider: String,
    pub settings: Settings,
    pub active: bool,
    #[serde(default)]
    pub remark: String,
}

#[derive(Serialize, Deserialize)]
struct Profiles {
    next_id: u64,
    default_id: Option<u64>,
    profiles: Vec<Profile>,
}

#[derive(Serialize)]
pub struct ProfileView {
    pub profile: Profile,
    pub has_key: bool,
}

#[derive(Serialize)]
pub struct ProfileList {
    pub default_id: Option<u64>,
    pub profiles: Vec<ProfileView>,
}

pub struct SettingsStore {
    directory: PathBuf,
    identifier: String,
    profile_write: std::sync::Mutex<()>,
    #[cfg(not(windows))]
    session_keys: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl SettingsStore {
    pub fn new(directory: PathBuf, identifier: String) -> Self {
        Self {
            directory,
            identifier,
            profile_write: Default::default(),
            #[cfg(not(windows))]
            session_keys: Default::default(),
        }
    }

    fn path(&self) -> PathBuf {
        self.directory.join("ai-settings.json")
    }

    fn profiles(&self) -> Result<Profiles, String> {
        let bytes = match fs::read(self.path()) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Profiles { next_id: 2, default_id: None, profiles: vec![] }),
            Err(_) => return Err("无法读取 AI 设置".into()),
        };
        if let Ok(mut profiles) = serde_json::from_slice::<Profiles>(&bytes) {
            let mut ids = std::collections::HashSet::new();
            for profile in &mut profiles.profiles {
                if profile.id == 0 || profile.id >= profiles.next_id || !ids.insert(profile.id) {
                    return Err("AI 设置文件中的配置编号无效".into());
                }
                profile.settings = profile.settings.clone().checked()?;
            }
            if profiles.default_id.is_some_and(|id| !profiles.profiles.iter().any(|profile| profile.id == id && profile.active)) {
                return Err("AI 设置文件中的默认配置无效".into());
            }
            return Ok(profiles);
        }
        let settings = serde_json::from_slice::<Settings>(&bytes).map_err(|_| "AI 设置文件无效")?;
        let settings = settings.checked()?;
        Ok(Profiles { next_id: 2, default_id: Some(1), profiles: vec![Profile {
            id: 1, name: settings.model.clone(), provider: "CUSTOM".into(), settings, active: true, remark: String::new(),
        }] })
    }

    pub fn list(&self) -> Result<ProfileList, String> {
        let profiles = self.profiles()?;
        let views = profiles.profiles.into_iter().map(|profile| {
            let has_key = self.key(profile.id, &profile.settings)?.is_some();
            Ok(ProfileView { profile, has_key })
        }).collect::<Result<_, String>>()?;
        Ok(ProfileList { default_id: profiles.default_id, profiles: views })
    }

    pub fn selected(&self, id: Option<u64>) -> Result<Option<Profile>, String> {
        let state = self.profiles()?;
        let fallback = state.profiles.iter().find(|profile| profile.active).map(|profile| profile.id);
        let id = id.or(state.default_id).or(fallback);
        Ok(state.profiles.into_iter().find(|profile| Some(profile.id) == id && profile.active))
    }

    pub fn view(&self) -> Result<View, String> {
        let Some(profile) = self.selected(None)? else { return Ok(View { settings: None, has_key: false }); };
        let has_key = self.key(profile.id, &profile.settings)?.is_some();
        Ok(View { settings: Some(profile.settings), has_key })
    }

    pub fn save(
        &self,
        settings: Settings,
        key: Option<String>,
        forget_key: bool,
    ) -> Result<View, String> {
        let mut profile = self.selected(None)?.unwrap_or_else(|| Profile { id: 0, name: settings.model.clone(), provider: "CUSTOM".into(),
            settings: settings.clone(), active: true, remark: String::new() });
        profile.settings = settings;
        self.save_profile(profile, key, forget_key, true)?;
        self.view()
    }

    pub fn save_profile(&self, mut profile: Profile, key: Option<String>, forget_key: bool, make_default: bool) -> Result<ProfileList, String> {
        let _guard = self.profile_write.lock().map_err(|_| "AI 配置不可用")?;
        profile.settings = profile.settings.checked()?;
        profile.name = profile.name.trim().to_string();
        if profile.name.is_empty() || profile.name.chars().count() > 100 || profile.name.chars().any(char::is_control) {
            return Err("请填写 1 至 100 个字符的配置名称".into());
        }
        check_key(key.as_deref())?;
        let mut state = self.profiles()?;
        if profile.id == 0 {
            profile.id = state.next_id;
            state.next_id = state.next_id.checked_add(1).ok_or("AI 模型数量超出限制")?;
        } else if !state.profiles.iter().any(|item| item.id == profile.id) {
            return Err("模型配置不存在".into());
        }
        if make_default && !profile.active { return Err("请启用配置后再设为默认".into()); }
        let key_change = if forget_key { Some(None) } else { key.as_deref().map(Some) };
        if let Some(item) = state.profiles.iter_mut().find(|item| item.id == profile.id) {
            *item = profile.clone();
        } else {
            state.profiles.push(profile.clone());
        }
        if profile.active && (make_default || state.default_id.is_none()) {
            state.default_id = Some(profile.id);
        } else if !profile.active && state.default_id == Some(profile.id) {
            state.default_id = state.profiles.iter().find(|item| item.active).map(|item| item.id);
        }
        self.write_with_key(&state, profile.id, &profile.settings, key_change)?;
        self.list()
    }

    pub fn change_profile(&self, id: u64, remove: bool) -> Result<ProfileList, String> {
        let _guard = self.profile_write.lock().map_err(|_| "AI 配置不可用")?;
        let mut state = self.profiles()?;
        let profile = state.profiles.iter().find(|item| item.id == id).ok_or("模型配置不存在")?.clone();
        if remove {
            state.profiles.retain(|item| item.id != id);
            if state.default_id == Some(id) { state.default_id = state.profiles.iter().find(|item| item.active).map(|item| item.id); }
        } else {
            if !profile.active { return Err("请启用配置后再设为默认".into()); }
            state.default_id = Some(id);
        }
        self.write_with_key(&state, id, &profile.settings, remove.then_some(None))?;
        self.list()
    }

    fn write_with_key(&self, state: &Profiles, id: u64, settings: &Settings, key_change: Option<Option<&str>>) -> Result<(), String> {
        let Some(new_key) = key_change else { return self.write_profiles(state); };
        let old_key = self.key(id, settings)?;
        let name = self.key_name(id, settings);
        self.set_key_for(&name, new_key)?;
        if let Err(error) = self.write_profiles(state) {
            self.set_key_for(&name, old_key.as_deref()).map_err(|_| "保存 AI 设置失败，无法恢复该配置原有凭据")?;
            return Err(error);
        }
        Ok(())
    }

    fn set_key_for(&self, name: &str, key: Option<&str>) -> Result<(), String> {
        #[cfg(windows)]
        { if let Some(key) = key { vault::write(name, key) } else { vault::delete(name) } }
        #[cfg(not(windows))]
        {
            let mut memory = self.session_keys.lock().map_err(|_| "凭据服务不可用")?;
            if let Some(key) = key { memory.insert(name.into(), key.into()); }
            else { memory.remove(name); }
            Ok(())
        }
    }

    fn write_profiles(&self, state: &Profiles) -> Result<(), String> {
        write_atomic(
            &self.path(),
            &serde_json::to_vec(state).map_err(|_| "无法编码 AI 设置")?,
        )
        .map_err(|_| "无法保存 AI 设置".into())
    }

    fn key_name(&self, id: u64, settings: &Settings) -> String {
        if id == 1 { format!("{}/AI/{}", self.identifier, settings.endpoint) }
        else { format!("{}/AI/{id}/{}", self.identifier, settings.endpoint) }
    }

    pub fn key(&self, id: u64, settings: &Settings) -> Result<Option<String>, String> {
        if id == 0 { return Ok(None); }
        let name = self.key_name(id, settings);
        #[cfg(windows)]
        {
            vault::read(&name)
        }
        #[cfg(not(windows))]
        {
            Ok(self
                .session_keys
                .lock()
                .map_err(|_| "凭据服务不可用")?
                .get(&name)
                .cloned())
        }
    }
}

impl Settings {
    pub fn checked(mut self) -> Result<Self, String> {
        self.model = self.model.trim().to_string();
        if self.model.is_empty()
            || self.model.chars().count() > 100
            || self.model.chars().any(char::is_control)
            || !(1..=120).contains(&self.timeout_seconds)
        {
            return Err("请输入有效模型名称和 1 至 120 秒超时".into());
        }
        if self.temperature.is_some_and(|value| !(0.0..=2.0).contains(&value)) ||
            self.max_tokens.is_some_and(|value| !(1..=131072).contains(&value)) ||
            self.reasoning_effort.as_deref().is_some_and(|value| !matches!(value, "off" | "low" | "medium" | "high")) {
            return Err("AI 采样或思考参数无效".into());
        }
        let url = Url::parse(self.endpoint.trim()).map_err(|_| "服务地址无效")?;
        let loopback = url
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|ip| ip.is_loopback());
        if !(url.scheme() == "https" || url.scheme() == "http" && loopback)
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.path().ends_with("/chat/completions")
            || url.as_str().len() > 512
        {
            return Err(
                "请输入完整的 HTTPS Chat Completions 地址，本机回环地址也可使用 HTTP".into(),
            );
        }
        self.endpoint = url.to_string();
        Ok(self)
    }
}

pub fn check_key(key: Option<&str>) -> Result<(), String> {
    if key.is_some_and(|key| key.is_empty() || key.len() > 2048 || !key.bytes().all(|byte| (33..=126).contains(&byte))) {
        return Err("API 密钥格式无效".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_settings_keep_credential_identity_and_select_new_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(directory.path().into(), "test.parity".into());
        fs::write(store.path(), r#"{"endpoint":"http://127.0.0.1:11434/v1/chat/completions","model":"legacy","timeout_seconds":30}"#).unwrap();
        let first = store.selected(None).unwrap().unwrap();
        assert_eq!(first.id, 1);
        assert_eq!(store.key_name(first.id, &first.settings), "test.parity/AI/http://127.0.0.1:11434/v1/chat/completions");
        let mut second = first.clone();
        second.id = 0;
        second.name = "新模型".into();
        second.settings.model = "new".into();
        let view = store.save_profile(second, None, false, true).unwrap();
        assert_eq!(view.profiles.len(), 2);
        let second = store.selected(None).unwrap().unwrap();
        assert_eq!(second.settings.model, "new");
        assert_ne!(store.key_name(first.id, &first.settings), store.key_name(second.id, &second.settings));
        assert_eq!(store.selected(Some(1)).unwrap().unwrap().settings.model, "legacy");
        store.change_profile(first.id, false).unwrap();
        assert_eq!(store.selected(None).unwrap().unwrap().id, first.id);
    }
}

#[cfg(windows)]
mod vault {
    use std::{ptr, slice};
    use windows_sys::Win32::{
        Foundation::{GetLastError, ERROR_NOT_FOUND},
        Security::Credentials::{
            CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
            CRED_TYPE_GENERIC,
        },
    };

    fn wide(name: &str) -> Vec<u16> {
        name.encode_utf16().chain(Some(0)).collect()
    }

    pub fn read(name: &str) -> Result<Option<String>, String> {
        let name = wide(name);
        let mut credential = ptr::null_mut();
        // CredReadW returns one allocated credential and takes a terminated UTF-16 target name.
        unsafe {
            if CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) == 0 {
                return if GetLastError() == ERROR_NOT_FOUND {
                    Ok(None)
                } else {
                    Err("无法读取 Windows 凭据".into())
                };
            }
            let value = &*credential;
            let bytes = if value.CredentialBlobSize == 0 {
                &[]
            } else {
                slice::from_raw_parts(value.CredentialBlob, value.CredentialBlobSize as usize)
            };
            let key = std::str::from_utf8(bytes)
                .map(str::to_string)
                .map_err(|_| "Windows 凭据内容无效");
            CredFree(credential.cast());
            key.map(Some).map_err(String::from)
        }
    }

    pub fn write(name: &str, key: &str) -> Result<(), String> {
        let mut name = wide(name);
        let mut label = wide("ClassmateHub");
        let mut bytes = key.as_bytes().to_vec();
        let item = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: name.as_mut_ptr(),
            UserName: label.as_mut_ptr(),
            CredentialBlobSize: bytes.len() as u32,
            CredentialBlob: bytes.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        // The credential and all of its buffers remain live for this call.
        let result = unsafe { CredWriteW(&item, 0) };
        bytes.fill(0);
        if result == 0 {
            Err("无法保存 Windows 凭据".into())
        } else {
            Ok(())
        }
    }

    pub fn delete(name: &str) -> Result<(), String> {
        let name = wide(name);
        // The target name is terminated and owned by this stack frame.
        unsafe {
            if CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) != 0
                || GetLastError() == ERROR_NOT_FOUND
            {
                Ok(())
            } else {
                Err("无法移除 Windows 凭据".into())
            }
        }
    }
}
