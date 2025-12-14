use rustbridge::*;
use rustbridge::log_colors::LogColors;
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use yaml_rust::YamlLoader;

/// Bridge configuration
#[derive(Debug)]
struct BridgeConfig {
    stratum_port: String,
    kaspad_address: String,
    prom_port: String,
    print_stats: bool,
    log_to_file: bool,
    health_check_port: String,
    block_wait_time: Duration,
    min_share_diff: u32,
    var_diff: bool,
    shares_per_min: u32,
    var_diff_stats: bool,
    extranonce_size: u8,
    pow2_clamp: bool,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            stratum_port: ":5555".to_string(),
            kaspad_address: "localhost:16110".to_string(),
            prom_port: ":2114".to_string(),
            print_stats: true,
            log_to_file: true,
            health_check_port: String::new(),
            block_wait_time: Duration::from_millis(1000), // 1 second (1000ms)
            min_share_diff: 8192,
            var_diff: true,
            shares_per_min: 20,
            var_diff_stats: false,
            extranonce_size: 0,
            pow2_clamp: false,
        }
    }
}

impl BridgeConfig {
    fn from_yaml(content: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let docs = YamlLoader::load_from_str(content)?;
        let doc = docs.get(0).ok_or("empty YAML document")?;
        
        let mut config = BridgeConfig::default();
        
        if let Some(port) = doc["stratum_port"].as_str() {
            config.stratum_port = if port.starts_with(':') {
                port.to_string()
            } else {
                format!(":{}", port)
            };
        }
        
        if let Some(addr) = doc["kaspad_address"].as_str() {
            config.kaspad_address = addr.to_string();
        }
        
        if let Some(port) = doc["prom_port"].as_str() {
            config.prom_port = if port.starts_with(':') {
                port.to_string()
            } else {
                format!(":{}", port)
            };
        }
        
        if let Some(stats) = doc["print_stats"].as_bool() {
            config.print_stats = stats;
        }
        
        if let Some(log) = doc["log_to_file"].as_bool() {
            config.log_to_file = log;
        }
        
        if let Some(port) = doc["health_check_port"].as_str() {
            config.health_check_port = port.to_string();
        }
        
        if let Some(diff) = doc["min_share_diff"].as_i64() {
            config.min_share_diff = diff as u32;
        }
        
        if let Some(vd) = doc["var_diff"].as_bool() {
            config.var_diff = vd;
        }
        
        if let Some(spm) = doc["shares_per_min"].as_i64() {
            config.shares_per_min = spm as u32;
        }
        
        if let Some(vds) = doc["var_diff_stats"].as_bool() {
            config.var_diff_stats = vds;
        }
        
        if let Some(ens) = doc["extranonce_size"].as_i64() {
            config.extranonce_size = ens as u8;
        }
        
        if let Some(clamp) = doc["pow2_clamp"].as_bool() {
            config.pow2_clamp = clamp;
        }
        
        // Parse block_wait_time from config (in milliseconds, convert to Duration)
        if let Some(bwt) = doc["block_wait_time"].as_i64() {
            // Value is in milliseconds
            config.block_wait_time = Duration::from_millis(bwt as u64);
        } else if let Some(bwt) = doc["block_wait_time"].as_f64() {
            // Also support float values (convert to milliseconds)
            config.block_wait_time = Duration::from_millis(bwt as u64);
        }
        
        Ok(config)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load config first to check if file logging is enabled
    let config_path = std::path::Path::new("config.yaml");
    let config = if config_path.exists() {
        let content = std::fs::read_to_string(config_path)?;
        BridgeConfig::from_yaml(&content)?
    } else {
        BridgeConfig::default()
    };
    
    // Initialize color support detection
    rustbridge::log_colors::LogColors::init();
    
    // Initialize tracing with WARN level by default (less verbose)
    // Can be overridden with RUST_LOG environment variable (e.g., RUST_LOG=info,debug)
    // To see more details, set RUST_LOG=info or RUST_LOG=debug
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            // Default: warn level, but allow info from rustbridge module for important messages
            EnvFilter::new("warn,rustbridge=info")
        });
    
    // Custom formatter that applies colors directly to the Writer (like tracing-subscriber does for levels)
    // We create two formatters: one with colors (for console) and one without (for file)
    use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
    use std::fmt;
    
    struct CustomFormatter {
        apply_colors: bool,
    }
    
    impl<S, N> FormatEvent<S, N> for CustomFormatter
    where
        S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
        N: for<'a> FormatFields<'a> + 'static,
    {
        fn format_event(
            &self,
            ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
            mut writer: Writer<'_>,
            event: &tracing::Event<'_>,
        ) -> fmt::Result {
            // Write level (with built-in ANSI colors from tracing-subscriber)
            let level = *event.metadata().level();
            write!(writer, "{:5} ", level)?;
            
            // Write target with capitalization
            let target = event.metadata().target();
            let formatted_target = if target.starts_with("rustbridge") {
                format!("rustbridge{}", &target["rustbridge".len()..])
            } else {
                target.to_string()
            };
            write!(writer, "{}: ", formatted_target)?;
            
            // Collect the message into a string first so we can analyze it for color patterns
            let mut message_buf = String::new();
            {
                let mut message_writer = Writer::new(&mut message_buf);
                ctx.format_fields(message_writer.by_ref(), event)?;
            }
            let mut message = message_buf;
            
            // Strip any existing ANSI codes from the message (from LogColors functions)
            // This regex-like approach removes ANSI escape sequences
            let mut cleaned_message = String::new();
            let mut chars = message.chars().peekable();
            while let Some(ch) = chars.next() {
                if ch == '\x1b' || ch == '\u{001b}' {
                    // Skip ANSI escape sequence: \x1b[ followed by numbers and letters until 'm'
                    if chars.peek() == Some(&'[') {
                        chars.next(); // consume '['
                        while let Some(&c) = chars.peek() {
                            if c == 'm' {
                                chars.next(); // consume 'm'
                                break;
                            }
                            chars.next();
                        }
                    }
                } else {
                    cleaned_message.push(ch);
                }
            }
            message = cleaned_message;
            
            // Apply colors based on message content patterns (only if this formatter has colors enabled)
            if self.apply_colors {
                if message.contains("[ASIC->BRIDGE]") {
                    write!(writer, "\x1b[96m{}", message)?; // Cyan
                    write!(writer, "\x1b[0m")?; // Reset
                } else if message.contains("[BRIDGE->ASIC]") {
                    write!(writer, "\x1b[92m{}", message)?; // Green
                    write!(writer, "\x1b[0m")?; // Reset
                } else if message.contains("[VALIDATION]") {
                    write!(writer, "\x1b[93m{}", message)?; // Yellow
                    write!(writer, "\x1b[0m")?; // Reset
                } else if message.contains("===== BLOCK") || message.contains("[BLOCK]") {
                    write!(writer, "\x1b[95m{}", message)?; // Magenta
                    write!(writer, "\x1b[0m")?; // Reset
                } else if message.contains("[API]") {
                    write!(writer, "\x1b[94m{}", message)?; // Blue
                    write!(writer, "\x1b[0m")?; // Reset
                } else if message.contains("Error") || message.contains("ERROR") {
                    write!(writer, "\x1b[91m{}", message)?; // Red
                    write!(writer, "\x1b[0m")?; // Reset
                } else if message.contains("----------------------------------") {
                    write!(writer, "\x1b[96m{}", message)?; // Bright Cyan for separator lines
                    write!(writer, "\x1b[0m")?; // Reset
                } else if message.contains("initializing bridge") {
                    write!(writer, "\x1b[92m{}", message)?; // Bright Green for initialization
                    write!(writer, "\x1b[0m")?; // Reset
                } else if message.contains("Starting RustBridge") {
                    write!(writer, "\x1b[92m{}", message)?; // Bright Green for startup
                    write!(writer, "\x1b[0m")?; // Reset
                } else if message.starts_with("\t") && message.contains(":") {
                    // Configuration lines - color the label part (e.g., "\tkaspad:          value")
                    if let Some(colon_pos) = message.find(':') {
                        // Find the end of the label (colon + whitespace)
                        let label_end = message[colon_pos + 1..]
                            .chars()
                            .take_while(|c| c.is_whitespace())
                            .count();
                        let label_end_pos = colon_pos + 1 + label_end;
                        let label = &message[..label_end_pos];
                        let value = &message[label_end_pos..];
                        write!(writer, "\x1b[94m{}\x1b[0m{}", label, value)?; // Blue for labels
                    } else {
                        write!(writer, "{}", message)?;
                    }
                } else {
                    write!(writer, "{}", message)?; // No color
                }
            } else {
                write!(writer, "{}", message)?;
            }
            
            writeln!(writer)
        }
    }
    
    // Setup file logging if enabled
    // Note: The file_guard must be kept alive for the lifetime of the program
    // to ensure logs are flushed to the file
    let _file_guard: Option<tracing_appender::non_blocking::WorkerGuard> = if config.log_to_file {
        // Create log file with timestamp
        use std::time::SystemTime;
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let log_filename = format!("rustbridge_{}.log", timestamp);
        let log_path = std::path::Path::new(".").join(&log_filename);
        
        // Use tracing-appender for file logging
        let file_appender = tracing_appender::rolling::never(".", &log_filename);
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        
        eprintln!("Logging to file: {}", log_path.display());
        
        // Setup logging with both console and file
        // Use default formatter for console (preserves ANSI codes) but with custom target formatting
        tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(LogColors::should_colorize()) // Enable ANSI colors for console conditionally
                    .event_format(CustomFormatter { apply_colors: LogColors::should_colorize() })
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(non_blocking)
                    .with_ansi(false) // Disable ANSI colors in file
                    .event_format(CustomFormatter { apply_colors: false })
            )
            .init();
        
        Some(_guard)
    } else {
        // Setup logging with console only
        tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(LogColors::should_colorize()) // Enable ANSI colors for console conditionally
                    .event_format(CustomFormatter { apply_colors: LogColors::should_colorize() })
                    // Use default formatter to preserve ANSI codes in messages
                    // .event_format(CustomFormatter)
            )
            .init();
        
        None
    };
    
    if !config_path.exists() {
        tracing::warn!("config.yaml not found, using defaults");
    }
    
    tracing::info!("----------------------------------");
    tracing::info!("initializing bridge");
    tracing::info!("\tkaspad:          {}", config.kaspad_address);
    tracing::info!("\tstratum:         {}", config.stratum_port);
    tracing::info!("\tprom:            {}", config.prom_port);
    tracing::info!("\tstats:           {}", config.print_stats);
    tracing::info!("\tlog:             {}", config.log_to_file);
    tracing::info!("\tmin diff:        {}", config.min_share_diff);
    tracing::info!("\tpow2 clamp:      {}", config.pow2_clamp);
    tracing::info!("\tvar diff:        {}", config.var_diff);
    tracing::info!("\tshares per min:  {}", config.shares_per_min);
    tracing::info!("\tvar diff stats:  {}", config.var_diff_stats);
    tracing::info!("\tblock wait:      {:?}", config.block_wait_time);
    tracing::info!("\textranonce:      auto-detected per client");
    tracing::info!("\thealth check:    {}", config.health_check_port);
    tracing::info!("----------------------------------");
    
    // Start Prometheus server if port is specified
    if !config.prom_port.is_empty() {
        let prom_port = config.prom_port.clone();
        tokio::spawn(async move {
            if let Err(e) = prom::start_prom_server(&prom_port).await {
                tracing::error!("Prometheus server error: {}", e);
            }
        });
    }
    
    // Start health check server if port is specified
    if !config.health_check_port.is_empty() {
        let health_port = config.health_check_port.clone();
        tokio::spawn(async move {
            use tokio::net::TcpListener;
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            
            if let Ok(listener) = TcpListener::bind(&health_port).await {
                tracing::info!("Health check server started on {}", health_port);
                loop {
                    if let Ok((mut stream, _)) = listener.accept().await {
                        let mut buffer = [0; 1024];
                        if stream.read(&mut buffer).await.is_ok() {
                            let response = "HTTP/1.1 200 OK\r\n\r\n";
                            let _ = stream.write_all(response.as_bytes()).await;
                        }
                    }
                }
            }
        });
    }
    
    // Create kaspa API client
    let kaspa_api = rustbridge::KaspaApi::new(
        config.kaspad_address.clone(),
        config.block_wait_time,
    ).await.map_err(|e| format!("Failed to create Kaspa API client: {}", e))?;

    // Create bridge config
    let bridge_config = rustbridge::BridgeConfig {
        stratum_port: config.stratum_port.clone(),
        kaspad_address: config.kaspad_address.clone(),
        prom_port: config.prom_port.clone(),
        print_stats: config.print_stats,
        log_to_file: config.log_to_file,
        health_check_port: config.health_check_port.clone(),
        block_wait_time: config.block_wait_time,
        min_share_diff: config.min_share_diff,
        var_diff: config.var_diff,
        shares_per_min: config.shares_per_min,
        var_diff_stats: config.var_diff_stats,
        extranonce_size: config.extranonce_size,
        pow2_clamp: config.pow2_clamp,
    };

    // Start block template listener with notifications + ticker fallback
    // This starts the notification-based block template listener with ticker fallback
    // We pass concrete_kaspa_api to listen_and_serve so it can use notifications
    
    // Start the bridge server
    tracing::info!("Starting RustBridge");
    rustbridge::listen_and_serve(bridge_config, Arc::clone(&kaspa_api), Some(kaspa_api)).await
        .map_err(|e| format!("Bridge server error: {}", e))?;
    
    Ok(())
}

