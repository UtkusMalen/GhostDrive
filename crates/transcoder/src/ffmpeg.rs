use std::path::PathBuf;
use std::process::Stdio;
use async_stream::try_stream;
use bytes::{Bytes, BytesMut};
use futures_core::Stream;
use tokio::process::{Child, Command};
use tokio::io::AsyncReadExt;
use tracing::{debug, error, info, instrument};
use ghostdrive_core::{StreamError, StreamResult};

#[derive(Debug, Clone)]
pub struct TranscodeOptions {
    pub video_codec: String,
    pub video_bitrate: String,
    pub audio_codec: String,
    pub format: String,
    pub resolution: Option<String>,
    pub frame_rate: Option<u32>,
}

impl Default for TranscodeOptions {
    fn default() -> Self {
        Self {
            video_codec: "libx264".to_string(),
            video_bitrate: "2M".to_string(),
            audio_codec: "aac".to_string(),
            format: "mpegts".to_string(),
            resolution: Some("1280x720".to_string()),
            frame_rate: Some(30),
        }
    }
}

pub struct Transcoder {
    process: Child,
}

impl Transcoder {
    /// Spawns a new FFmpeg process to transcode the input file
    /// Returns immediately with the Transcoder handle
    #[instrument(skip(options))]
    pub async fn new(input_path: PathBuf, options: TranscodeOptions) -> StreamResult<Self> {
        // Validate FFmpeg installation
        match Command::new("ffmpeg").arg("-version").output().await {
            Ok(output) if output.status.success() => {
                debug!("FFmpeg detected successfully");
            }
            _ => {
                return Err(StreamError::Transcode(
                    "FFmpeg not found. Please ensure ffmpeg is installed and in PATH".to_string()
                ));
            }
        }

        if !input_path.exists() {
            return Err(StreamError::FileNotFound(input_path));
        }

        // Build command
        let mut cmd = Command::new("ffmpeg");

        // Input options
        cmd.arg("-hide_banner")
            .arg("-loglevel").arg("error")
            .arg("-i").arg(&input_path);

        // Video options
        cmd.arg("-c:v").arg(&options.video_codec)
            .arg("-b:v").arg(&options.video_bitrate);

        if let Some(res) = &options.resolution {
            cmd.arg("-s").arg(res);
        }

        if let Some(fps) = options.frame_rate {
            cmd.arg("-r").arg(fps.to_string());
        }

        // Optimization for latency (zerolatency tuning for x264)
        if options.video_codec == "libx264" {
            cmd.arg("-preset").arg("veryfast")
                .arg("-tune").arg("zerolatency");
        }

        // Audio options
        cmd.arg("-c:a").arg(&options.audio_codec);

        // Output options (Stdout pipe)
        cmd.arg("-f").arg(&options.format)
            .arg("pipe:1");

        // Cleanup configuration
        cmd.kill_on_drop(true);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped()); // Capture stderr to debug failures

        // Spawn
        info!("Spawning FFmpeg for {:?}", input_path);
        debug!("Command: {:?}", cmd);

        let process = cmd.spawn()
            .map_err(|e| StreamError::Io(e))?;

        Ok(Self { process })
    }
    
    /// Take the stdout handle from the child process
    /// Returns None if it was already taken
    pub fn stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.process.stdout.take()
    }
    
    /// Wait for the process to complete and check status
    /// If non-zero exit code, reads stderr for details
    pub async fn wait(mut self) -> StreamResult<()> {
        let status = self.process.wait().await.map_err(StreamError::Io)?;
        
        if !status.success() {
            // Attempt to read stderr if available
            let mut err_msg = String::new();
            if let Some(mut stderr) = self.process.stderr.take() {
                let _ = stderr.read_to_string(&mut err_msg).await;
            }
            
            error!("FFmpeg exited with error: {}", err_msg);
            return Err(StreamError::Transcode(format!(
                "Process exited with code {:?}: {}",
                status.code(),
                err_msg
            )));
        }
        
        Ok(())
    }

    /// Stream the output of the transcoding process in chunks
    ///
    /// This consumes the Transcoder instance. The underlying FFmpeg process
    /// will be killed when stream is dropped, or waited upon when EOF if reached
    pub fn stream_chunks(
        mut self,
        chunk_size: usize
    ) -> impl Stream<Item = Result<Bytes, StreamError>> {
        try_stream! {
            // Take stdout out of the process struct
            let mut stdout = self.stdout()
                .ok_or_else(|| StreamError::Transcode("Stdout already taken".to_string()))?;

            let mut buffer = BytesMut::with_capacity(chunk_size);

            loop {
                // Ensure we have capacity to read
                if buffer.capacity() < chunk_size {
                    buffer.reserve(chunk_size);
                }

                // Read from pipe directly into buffer
                let n = stdout.read_buf(&mut buffer).await.map_err(StreamError::Io)?;

                if n == 0 {
                    // EOF reached. Verify process exit status
                    // Must drop stdout before waiting
                    drop(stdout);
                    self.wait().await?;
                    break;
                }

                // Yield the chunk
                // split() returns the filled part and leaves 'buffer' empty but with some capacity
                yield buffer.split().freeze();
            }
        }
    }
}