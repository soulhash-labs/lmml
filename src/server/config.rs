use serde::{Deserialize, Serialize};

/// Server configuration, persisted in config.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub port: u16,
    pub context_size: u32,
    pub gpu_layers: u32,
    pub threads: u32,
    pub batch_size: u32,
    pub extra_args: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            port: 8080,
            context_size: 8192,
            gpu_layers: 99,
            threads: 0,
            batch_size: 512,
            extra_args: Vec::new(),
        }
    }
}

impl ServerConfig {
    /// Build the command-line args vector for llama-server.
    pub fn to_args(&self, model_path: &str) -> Vec<String> {
        let mut args = vec![
            "-m".to_string(),
            model_path.to_string(),
            "--port".to_string(),
            self.port.to_string(),
            "-c".to_string(),
            self.context_size.to_string(),
            "-ngl".to_string(),
            self.gpu_layers.to_string(),
        ];
        if self.threads > 0 {
            args.push("-t".to_string());
            args.push(self.threads.to_string());
        }
        args.push("-b".to_string());
        args.push(self.batch_size.to_string());
        args.extend(self.extra_args.iter().cloned());
        args
    }

    /// Display config in human-readable form.
    pub fn display(&self, model_path: &str) -> Vec<String> {
        vec![
            format!("Port:       {}", self.port),
            format!("Context:    {}", self.context_size),
            format!("GPU layers: {}", self.gpu_layers),
            format!(
                "Threads:    {}",
                if self.threads == 0 {
                    "auto".to_string()
                } else {
                    self.threads.to_string()
                }
            ),
            format!("Batch:      {}", self.batch_size),
            format!("Model:      {model_path}"),
        ]
    }
}
