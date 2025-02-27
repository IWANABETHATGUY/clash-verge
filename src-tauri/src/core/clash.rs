use super::{PrfEnhancedResult, Profiles, Verge};
use crate::utils::{config, dirs, help};
use anyhow::{bail, Result};
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::{collections::HashMap, time::Duration};
use tauri::api::process::{Command, CommandChild, CommandEvent};
use tauri::Window;
use tokio::time::sleep;

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct ClashInfo {
  /// clash sidecar status
  pub status: String,

  /// clash core port
  pub port: Option<String>,

  /// same as `external-controller`
  pub server: Option<String>,

  /// clash secret
  pub secret: Option<String>,
}

pub struct Clash {
  /// maintain the clash config
  pub config: Mapping,

  /// some info
  pub info: ClashInfo,

  /// clash sidecar
  pub sidecar: Option<CommandChild>,

  /// save the main window
  pub window: Option<Window>,
}

impl Clash {
  pub fn new() -> Clash {
    let config = Clash::read_config();
    let info = Clash::get_info(&config);

    Clash {
      config,
      info,
      sidecar: None,
      window: None,
    }
  }

  /// get clash config
  fn read_config() -> Mapping {
    config::read_yaml::<Mapping>(dirs::clash_path())
  }

  /// save the clash config
  fn save_config(&self) -> Result<()> {
    config::save_yaml(
      dirs::clash_path(),
      &self.config,
      Some("# Default Config For Clash Core\n\n"),
    )
  }

  /// parse the clash's config.yaml
  /// get some information
  fn get_info(clash_config: &Mapping) -> ClashInfo {
    let key_port_1 = Value::String("port".to_string());
    let key_port_2 = Value::String("mixed-port".to_string());
    let key_server = Value::String("external-controller".to_string());
    let key_secret = Value::String("secret".to_string());

    let port = match clash_config.get(&key_port_1) {
      Some(value) => match value {
        Value::String(val_str) => Some(val_str.clone()),
        Value::Number(val_num) => Some(val_num.to_string()),
        _ => None,
      },
      _ => None,
    };
    let port = match port {
      Some(_) => port,
      None => match clash_config.get(&key_port_2) {
        Some(value) => match value {
          Value::String(val_str) => Some(val_str.clone()),
          Value::Number(val_num) => Some(val_num.to_string()),
          _ => None,
        },
        _ => None,
      },
    };

    let server = match clash_config.get(&key_server) {
      Some(value) => match value {
        Value::String(val_str) => Some(val_str.clone()),
        _ => None,
      },
      _ => None,
    };
    let secret = match clash_config.get(&key_secret) {
      Some(value) => match value {
        Value::String(val_str) => Some(val_str.clone()),
        Value::Bool(val_bool) => Some(val_bool.to_string()),
        Value::Number(val_num) => Some(val_num.to_string()),
        _ => None,
      },
      _ => None,
    };

    ClashInfo {
      status: "init".into(),
      port,
      server,
      secret,
    }
  }

  /// save the main window
  pub fn set_window(&mut self, win: Option<Window>) {
    self.window = win;
  }

  /// run clash sidecar
  pub fn run_sidecar(&mut self) -> Result<()> {
    let app_dir = dirs::app_home_dir();
    let app_dir = app_dir.as_os_str().to_str().unwrap();

    match Command::new_sidecar("clash") {
      Ok(cmd) => match cmd.args(["-d", app_dir]).spawn() {
        Ok((mut rx, cmd_child)) => {
          self.sidecar = Some(cmd_child);

          // clash log
          tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
              match event {
                CommandEvent::Stdout(line) => log::info!("[clash]: {}", line),
                CommandEvent::Stderr(err) => log::error!("[clash]: {}", err),
                _ => {}
              }
            }
          });
          Ok(())
        }
        Err(err) => bail!(err.to_string()),
      },
      Err(err) => bail!(err.to_string()),
    }
  }

  /// drop clash sidecar
  pub fn drop_sidecar(&mut self) -> Result<()> {
    if let Some(sidecar) = self.sidecar.take() {
      sidecar.kill()?;
    }
    Ok(())
  }

  /// restart clash sidecar
  /// should reactivate profile after restart
  pub fn restart_sidecar(&mut self, profiles: &mut Profiles) -> Result<()> {
    self.update_config();
    self.drop_sidecar()?;
    self.run_sidecar()?;
    self.activate(profiles, false)
  }

  /// update the clash info
  pub fn update_config(&mut self) {
    self.config = Clash::read_config();
    self.info = Clash::get_info(&self.config);
  }

  /// patch update the clash config
  pub fn patch_config(
    &mut self,
    patch: Mapping,
    verge: &mut Verge,
    profiles: &mut Profiles,
  ) -> Result<()> {
    for (key, value) in patch.iter() {
      let value = value.clone();
      let key_str = key.as_str().clone().unwrap_or("");

      // restart the clash
      if key_str == "mixed-port" {
        self.restart_sidecar(profiles)?;

        let port = if value.is_number() {
          match value.as_i64().clone() {
            Some(num) => Some(format!("{num}")),
            None => None,
          }
        } else {
          match value.as_str().clone() {
            Some(num) => Some(num.into()),
            None => None,
          }
        };
        verge.init_sysproxy(port);
      }

      self.config.insert(key.clone(), value);
    }
    self.save_config()
  }

  /// enable tun mode
  /// only revise the config and restart the
  pub fn tun_mode(&mut self, enable: bool) -> Result<()> {
    let tun_key = Value::String("tun".into());
    let tun_val = self.config.get(&tun_key);

    let mut new_val = Mapping::new();

    if tun_val.is_some() && tun_val.as_ref().unwrap().is_mapping() {
      new_val = tun_val.as_ref().unwrap().as_mapping().unwrap().clone();
    }

    macro_rules! revise {
      ($map: expr, $key: expr, $val: expr) => {
        let ret_key = Value::String($key.into());
        if $map.contains_key(&ret_key) {
          $map[&ret_key] = $val;
        } else {
          $map.insert(ret_key, $val);
        }
      };
    }

    macro_rules! append {
      ($map: expr, $key: expr, $val: expr) => {
        let ret_key = Value::String($key.into());
        if !$map.contains_key(&ret_key) {
          $map.insert(ret_key, $val);
        }
      };
    }

    revise!(new_val, "enable", Value::from(enable));
    append!(new_val, "stack", Value::from("gvisor"));
    append!(new_val, "auto-route", Value::from(true));
    append!(new_val, "auto-detect-interface", Value::from(true));

    revise!(self.config, "tun", Value::from(new_val));

    self.save_config()
  }

  /// activate the profile
  /// generate a new profile to the temp_dir
  /// then put the path to the clash core
  fn _activate(info: ClashInfo, config: Mapping) -> Result<()> {
    let temp_path = dirs::profiles_temp_path();
    config::save_yaml(temp_path.clone(), &config, Some("# Clash Verge Temp File"))?;

    tauri::async_runtime::spawn(async move {
      let server = info.server.unwrap();
      let server = format!("http://{server}/configs");

      let mut headers = HeaderMap::new();
      headers.insert("Content-Type", "application/json".parse().unwrap());

      if let Some(secret) = info.secret.as_ref() {
        let secret = format!("Bearer {}", secret.clone()).parse().unwrap();
        headers.insert("Authorization", secret);
      }

      let mut data = HashMap::new();
      data.insert("path", temp_path.as_os_str().to_str().unwrap());

      // retry 5 times
      for _ in 0..5 {
        match reqwest::ClientBuilder::new().no_proxy().build() {
          Ok(client) => {
            let builder = client.put(&server).headers(headers.clone()).json(&data);

            match builder.send().await {
              Ok(resp) => {
                if resp.status() != 204 {
                  log::error!("failed to activate clash for status \"{}\"", resp.status());
                }
                // do not retry
                break;
              }
              Err(err) => log::error!("failed to activate for `{err}`"),
            }
          }
          Err(err) => log::error!("failed to activate for `{err}`"),
        }
        sleep(Duration::from_millis(500)).await;
      }
    });

    Ok(())
  }

  /// enhanced profiles mode
  /// only change the enhanced profiles
  pub fn activate_enhanced(&self, profiles: &Profiles, delay: bool) -> Result<()> {
    if self.window.is_none() {
      bail!("failed to get the main window");
    }

    let win = self.window.clone().unwrap();
    let event_name = help::get_uid("e");
    let event_name = format!("enhanced-cb-{event_name}");

    let info = self.info.clone();
    let mut config = self.config.clone();

    // generate the payload
    let payload = profiles.gen_enhanced(event_name.clone())?;

    win.once(&event_name, move |event| {
      if let Some(result) = event.payload() {
        let result: PrfEnhancedResult = serde_json::from_str(result).unwrap();

        if let Some(data) = result.data {
          // all of these can not be revised by script
          // http/https/socks port should be under control
          let not_allow: Vec<Value> = vec![
            "port",
            "socks-port",
            "mixed-port",
            "allow-lan",
            "mode",
            "external-controller",
            "secret",
            "log-level",
          ]
          .iter()
          .map(|&i| Value::from(i))
          .collect();

          for (key, value) in data.into_iter() {
            if not_allow.iter().find(|&i| i == &key).is_none() {
              config.insert(key, value);
            }
          }

          Self::_activate(info, config).unwrap();
        }

        log::info!("profile enhanced status {}", result.status);

        result.error.map(|error| log::error!("{error}"));
      }
    });

    tauri::async_runtime::spawn(async move {
      // wait the window setup during resolve app
      if delay {
        sleep(Duration::from_secs(2)).await;
      }
      win.emit("script-handler", payload).unwrap();
    });

    Ok(())
  }

  /// activate the profile
  /// auto activate enhanced profile
  pub fn activate(&self, profiles: &Profiles, delay: bool) -> Result<()> {
    let gen_map = profiles.gen_activate()?;
    let info = self.info.clone();
    let mut config = self.config.clone();

    for (key, value) in gen_map.into_iter() {
      config.insert(key, value);
    }

    Self::_activate(info, config)?;
    self.activate_enhanced(profiles, delay)
  }
}

impl Default for Clash {
  fn default() -> Self {
    Clash::new()
  }
}

impl Drop for Clash {
  fn drop(&mut self) {
    if let Err(err) = self.drop_sidecar() {
      log::error!("{err}");
    }
  }
}
