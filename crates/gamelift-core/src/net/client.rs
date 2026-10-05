//! 目标端接收：多连接并发拉取、逐块校验、断点续传与原子落盘
//!
//! 内容先落到同盘暂存目录，全部通过校验后才改名到目标目录
//! 续传状态与暂存目录分离，避免状态文件跟着内容一起被搬进游戏目录

use std::collections::VecDeque;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{Error, Result, TransferStats};

use super::diff;
use super::frame::{read_frame, write_frame, Frame, KIND_HASHES, KIND_PLAN};
use super::paths::{join_within, sanitize_relative};
use super::protocol::{self, ClaimFile, FileEntry, Message, RejectKind, PROTOCOL_VERSION};
use super::resume::{self, FileState, TransferState};
use super::{clamp_chunk_bytes, clamp_streams, io_error, lock, DIGEST_BYTES, STAGING_PREFIX};

/// 连接超时
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 读写超时，避免对端失联时永久挂住
const IO_TIMEOUT: Duration = Duration::from_mins(1);

/// 单块最大尝试次数
const MAX_ATTEMPTS: u32 = 3;

/// 重试前等待
const RETRY_DELAY: Duration = Duration::from_millis(200);

/// 进度回调间隔
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// 状态落盘间隔
const STATE_FLUSH_INTERVAL: Duration = Duration::from_secs(2);

/// 空间预检余量
const SPACE_MARGIN: u64 = 512 * 1024 * 1024;

/// 内容落盘布局
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// 内容先落暂存目录，整体改名为 `<目标父目录>/<内容根名>`，保持既有语义
    #[default]
    Wrapper,
    /// 内容先落暂存目录，再把顶层条目逐个移入目标目录，用于"直接落到我选的文件夹"
    IntoDestination,
}

/// 接收参数
#[derive(Debug, Clone)]
pub struct RecvOptions {
    /// 源端地址
    pub peer: SocketAddr,
    /// 配对码
    pub pairing: Option<String>,
    /// 请求的内容名
    pub want: String,
    /// 目标父目录，内容会落到它下面的子目录
    pub dest_parent: PathBuf,
    /// 认领文件的根目录，`None` 表示不写认领文件
    pub claim_root: Option<PathBuf>,
    /// 并发连接数
    pub streams: usize,
    /// 分块大小
    pub chunk_bytes: u32,
    /// 取消标志，置位后停止拉取并保留续传状态
    pub cancel: Option<Arc<AtomicBool>>,
    /// 目标目录已存在时是否覆盖
    pub force: bool,
    /// 目标机上已有副本的根目录，给出后启用差异传输
    pub old_root: Option<PathBuf>,
    /// 内容落盘布局
    pub layout: Layout,
}

/// 接收结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecvOutcome {
    /// 内容最终目录
    pub root: PathBuf,
    /// 本次会话从网络收到并写盘的字节数
    pub bytes_received: u64,
    /// 内容总字节数
    pub bytes_total: u64,
    /// 续传时跳过的字节数
    pub bytes_resumed: u64,
    /// 写出的认领文件
    pub claim_files: Vec<PathBuf>,
}

/// 源端清单的本地副本
#[derive(Debug, Clone)]
struct RemoteManifest {
    title: String,
    root_name: String,
    files: Vec<FileEntry>,
    total_bytes: u64,
    claim_files: Vec<ClaimFile>,
}

/// 单个文件的差异计划与本地旧块位置
#[derive(Debug, Clone)]
struct DiffPlan {
    /// 源端给出的分块计划
    plan: Vec<diff::PlanEntry>,
    /// 本地旧副本的分块索引，命中时按其中的位置拷贝
    local: Vec<diff::ChunkHash>,
}

/// 待拉取的一个分块
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkRef {
    /// 文件在清单中的下标
    file: u32,
    /// 文件内的分块序号
    chunk_index: u64,
    /// 块内起始偏移
    offset: u64,
    /// 块长度
    len: u32,
}

/// 单个文件的实时进度
#[derive(Debug, Clone)]
struct FileProgress {
    /// 完成位图
    bitmap: Vec<u8>,
    /// 分块总数
    chunks: u64,
}

/// 全局实时进度
#[derive(Debug, Clone)]
struct Progress {
    /// 各文件进度
    files: Vec<FileProgress>,
    /// 已完成字节数，含续传跳过的部分
    done_bytes: u64,
    /// 本次会话从网络收到的字节数
    received_bytes: u64,
}

/// 一次传输的会话上下文
struct TransferSession {
    manifest: RemoteManifest,
    state_path: PathBuf,
    base: TransferState,
    cancel: Option<Arc<AtomicBool>>,
}

/// 工作线程共享的上下文
struct WorkerContext {
    peer: SocketAddr,
    pairing: Option<String>,
    want: String,
    queue: Mutex<VecDeque<ChunkRef>>,
    progress: Arc<Mutex<Progress>>,
    staged_paths: Vec<PathBuf>,
    abort: AtomicBool,
    cancel: Option<Arc<AtomicBool>>,
}

impl WorkerContext {
    /// 是否应当停止拉取
    fn stopped(&self) -> bool {
        self.abort.load(Ordering::Relaxed)
            || self
                .cancel
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    /// 取下一个待拉取分块
    fn next_chunk(&self) -> Option<ChunkRef> {
        lock(&self.queue).pop_front()
    }
}

/// 接收一次传输，内容先落暂存目录，全部完成后改名到目标目录
///
/// # Errors
///
/// 握手被拒、空间不足、连续校验失败或 IO 出错时返回错误，续传状态会保留
#[allow(clippy::too_many_lines)]
pub fn recv(
    options: &RecvOptions,
    progress: &mut dyn FnMut(&TransferStats),
) -> Result<RecvOutcome> {
    let chunk_bytes = u64::from(clamp_chunk_bytes(options.chunk_bytes));
    let streams = clamp_streams(options.streams);
    let (mut control, manifest) =
        open_control(options.peer, options.pairing.as_deref(), &options.want)?;
    if manifest.files.is_empty() {
        return Err(Error::Protocol("源端清单里没有文件".to_owned()));
    }
    let root_name = sanitize_relative(&manifest.root_name)?;
    if root_name.components().count() != 1 {
        return Err(Error::PathEscape(manifest.root_name.clone()));
    }
    let staging = options
        .dest_parent
        .join(format!("{STAGING_PREFIX}{}", root_name.display()));
    let final_root = join_within(&options.dest_parent, &root_name)?;
    let state_path = resume::state_path(&options.dest_parent, &manifest.root_name);
    std::fs::create_dir_all(&options.dest_parent).map_err(io_error)?;
    if options.layout == Layout::Wrapper {
        check_existing(options, &final_root)?;
    }
    check_space(&options.dest_parent, manifest.total_bytes)?;

    let plans = plan_diff(&mut control, options, &manifest)?;
    drop(control);

    let resumable = staging.is_dir() && state_path.is_file();
    if !resumable {
        let _ = std::fs::remove_dir_all(&staging);
        TransferState::remove(&state_path)?;
    }
    std::fs::create_dir_all(&staging).map_err(io_error)?;
    let staged_paths = prepare_staging(&staging, &manifest.files)?;
    let base = load_state(&state_path, &manifest, chunk_bytes, resumable, &plans)?;
    let progress_cell = Arc::new(Mutex::new(build_progress(&base, &staged_paths)));
    copy_matched_chunks(options, &base, &plans, &staged_paths, &progress_cell)?;
    let queue = Mutex::new(build_queue(&base, &lock(&progress_cell))?);
    let context = Arc::new(WorkerContext {
        peer: options.peer,
        pairing: options.pairing.clone(),
        want: options.want.clone(),
        queue,
        progress: Arc::clone(&progress_cell),
        staged_paths,
        abort: AtomicBool::new(false),
        cancel: options.cancel.clone(),
    });
    let session = TransferSession {
        manifest,
        state_path,
        base,
        cancel: options.cancel.clone(),
    };

    let handles = spawn_workers(&context, streams)?;
    monitor(&handles, &progress_cell, &session, progress)?;
    let (received, done, cancelled) = join_workers(handles, &progress_cell, &session)?;
    if cancelled {
        return Err(Error::Cancelled);
    }
    finish(options, &session, &staging, &final_root, received, done)
}

/// 建立工作线程
fn spawn_workers(
    context: &Arc<WorkerContext>,
    streams: usize,
) -> Result<Vec<JoinHandle<Result<()>>>> {
    let mut handles = Vec::with_capacity(streams);
    for _ in 0..streams {
        let context = Arc::clone(context);
        let handle = thread::Builder::new()
            .name("gamelift-recv".to_owned())
            .spawn(move || worker_loop(&context))
            .map_err(io_error)?;
        handles.push(handle);
    }
    Ok(handles)
}

/// 采样进度并定期落盘状态，直到全部工作线程结束
fn monitor(
    handles: &[JoinHandle<Result<()>>],
    progress_cell: &Mutex<Progress>,
    session: &TransferSession,
    progress: &mut dyn FnMut(&TransferStats),
) -> Result<()> {
    let started = Instant::now();
    let mut last_flush = Instant::now();
    loop {
        thread::sleep(PROGRESS_INTERVAL);
        let stats = {
            let guard = lock(progress_cell);
            make_stats(&guard, session.manifest.total_bytes, started.elapsed())
        };
        progress(&stats);
        if last_flush.elapsed() >= STATE_FLUSH_INTERVAL {
            snapshot_state(&session.base, progress_cell).save(&session.state_path)?;
            last_flush = Instant::now();
        }
        if handles.iter().all(JoinHandle::is_finished) {
            break;
        }
    }
    Ok(())
}

/// 等待工作线程结束并汇总结果
///
/// # Errors
///
/// 状态落盘失败时返回 [`Error::Io`]
fn join_workers(
    handles: Vec<JoinHandle<Result<()>>>,
    progress_cell: &Mutex<Progress>,
    session: &TransferSession,
) -> Result<(u64, u64, bool)> {
    let mut first_error: Option<Error> = None;
    for handle in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => first_error = first_error.or(Some(err)),
            Err(_) => {
                first_error = first_error.or(Some(Error::Io("接收线程异常退出".to_owned())));
            }
        }
    }
    let (received, done) = {
        let guard = lock(progress_cell);
        (guard.received_bytes, guard.done_bytes)
    };
    snapshot_state(&session.base, progress_cell).save(&session.state_path)?;
    let cancelled = session
        .cancel
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::Relaxed));
    if let Some(err) = first_error {
        return Err(err);
    }
    Ok((received, done, cancelled))
}

/// 校验暂存内容并在成功后提交
fn finish(
    options: &RecvOptions,
    session: &TransferSession,
    staging: &Path,
    final_root: &Path,
    received: u64,
    done: u64,
) -> Result<RecvOutcome> {
    verify_staged(staging, &session.manifest.files)?;
    let root = match options.layout {
        Layout::Wrapper => {
            commit(staging, final_root)?;
            final_root.to_path_buf()
        }
        Layout::IntoDestination => {
            commit_entries(staging, &options.dest_parent, options.force)?;
            options.dest_parent.clone()
        }
    };
    let claim_files = write_claim_files(options, &session.manifest.claim_files)?;
    TransferState::remove(&session.state_path)?;
    Ok(RecvOutcome {
        root,
        bytes_received: received,
        bytes_total: session.manifest.total_bytes,
        bytes_resumed: done.saturating_sub(received),
        claim_files,
    })
}

/// 把暂存目录里的顶层条目逐个移入目标目录，同名冲突按 `force` 处理
fn commit_entries(staging: &Path, destination: &Path, force: bool) -> Result<()> {
    let entries: Vec<PathBuf> = std::fs::read_dir(staging)
        .map_err(io_error)?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    if !force {
        for path in &entries {
            let target = destination.join(path.file_name().unwrap_or_default());
            if target.exists() {
                return Err(Error::Io(format!(
                    "目标位置已有同名内容: {}，确认覆盖请加 --force",
                    target.display()
                )));
            }
        }
    }
    for path in &entries {
        let target = destination.join(path.file_name().unwrap_or_default());
        if target.exists() {
            let backup = target.with_file_name(format!(
                ".gamelift-old-{}",
                path.file_name().map_or_else(
                    || "item".to_owned(),
                    |name| name.to_string_lossy().into_owned()
                )
            ));
            let _ = std::fs::remove_dir_all(&backup);
            let _ = std::fs::remove_file(&backup);
            std::fs::rename(&target, &backup).map_err(io_error)?;
            if let Err(err) = std::fs::rename(path, &target) {
                let _ = std::fs::rename(&backup, &target);
                return Err(Error::Io(format!("移入 {} 失败: {err}", target.display())));
            }
            let _ = std::fs::remove_dir_all(&backup);
        } else {
            std::fs::rename(path, &target)
                .map_err(|err| Error::Io(format!("移入 {} 失败: {err}", target.display())))?;
        }
    }
    let _ = std::fs::remove_dir_all(staging);
    Ok(())
}

/// 目标目录已存在时必须显式允许覆盖，真正的替换放到提交阶段
fn check_existing(options: &RecvOptions, final_root: &Path) -> Result<()> {
    if final_root.exists() && !options.force {
        return Err(Error::Io(format!(
            "目标目录已存在: {}，确认覆盖请加 --force",
            final_root.display()
        )));
    }
    Ok(())
}

/// 把暂存目录提交为目标目录，先挪开旧目录，失败再挪回来
fn commit(staging: &Path, final_root: &Path) -> Result<()> {
    if !final_root.exists() {
        return std::fs::rename(staging, final_root)
            .map_err(|err| Error::Io(format!("提交 {} 失败: {err}", final_root.display())));
    }
    let backup = final_root.with_file_name(format!(
        ".gamelift-old-{}",
        final_root.file_name().map_or_else(
            || "target".to_owned(),
            |name| name.to_string_lossy().into_owned()
        )
    ));
    let _ = std::fs::remove_dir_all(&backup);
    std::fs::rename(final_root, &backup)
        .map_err(|err| Error::Io(format!("挪开旧目录失败: {err}")))?;
    match std::fs::rename(staging, final_root) {
        Ok(()) => {
            let _ = std::fs::remove_dir_all(&backup);
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::rename(&backup, final_root);
            Err(Error::Io(format!(
                "提交 {} 失败: {err}",
                final_root.display()
            )))
        }
    }
}

/// 空间预检
fn check_space(parent: &Path, total_bytes: u64) -> Result<()> {
    let free = crate::link::free_bytes_at(parent)?;
    let needed = total_bytes.saturating_add(SPACE_MARGIN);
    if free < needed {
        return Err(Error::InsufficientSpace(needed - free));
    }
    Ok(())
}

/// 建立暂存目录结构并落空文件，空文件也要真实存在
fn prepare_staging(staging: &Path, files: &[FileEntry]) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::with_capacity(files.len());
    for file in files {
        let relative = sanitize_relative(&file.path)?;
        let path = join_within(staging, &relative)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io_error)?;
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(io_error)?;
        paths.push(path);
    }
    Ok(paths)
}

/// 读取或初始化续传状态
fn load_state(
    state_path: &Path,
    manifest: &RemoteManifest,
    chunk_bytes: u64,
    resumable: bool,
    plans: &[Option<DiffPlan>],
) -> Result<TransferState> {
    if resumable {
        if let Some(existing) = TransferState::load(state_path)? {
            if state_matches(&existing, manifest, chunk_bytes, plans) {
                return Ok(existing);
            }
        }
    }
    let state = initial_state(manifest, chunk_bytes, plans);
    state.save(state_path)?;
    Ok(state)
}

/// 续传状态是否与本次清单和差异计划一致
fn state_matches(
    state: &TransferState,
    manifest: &RemoteManifest,
    chunk_bytes: u64,
    plans: &[Option<DiffPlan>],
) -> bool {
    if state.chunk_bytes != chunk_bytes
        || state.title != manifest.title
        || state.files.len() != manifest.files.len()
    {
        return false;
    }
    manifest.files.iter().enumerate().all(|(index, entry)| {
        let Some(declared) = state.files.get(index) else {
            return false;
        };
        declared.path == entry.path
            && declared.size == entry.size
            && declared.spans == expected_spans(index, plans)
    })
}

/// 差异计划对应的分块边界，全量传输时为空
fn expected_spans(index: usize, plans: &[Option<DiffPlan>]) -> Vec<resume::Span> {
    plans
        .get(index)
        .and_then(|plan| plan.as_ref())
        .map(|plan| {
            plan.plan
                .iter()
                .map(|entry| resume::Span {
                    offset: entry.span.offset,
                    len: entry.span.len,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 按清单与差异计划构造空状态
fn initial_state(
    manifest: &RemoteManifest,
    chunk_bytes: u64,
    plans: &[Option<DiffPlan>],
) -> TransferState {
    let files = manifest
        .files
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let spans = expected_spans(index, plans);
            let chunks = if spans.is_empty() {
                resume::chunk_count(entry.size, chunk_bytes)
            } else {
                u64::try_from(spans.len()).unwrap_or(u64::MAX)
            };
            FileState {
                path: entry.path.clone(),
                size: entry.size,
                bitmap: resume::encode_bitmap(&vec![0u8; resume::bitmap_bytes(chunks)]),
                spans,
            }
        })
        .collect();
    TransferState::new(manifest.title.clone(), chunk_bytes, files)
}

/// 由续传状态还原实时进度，磁盘上文件比声明的短时清掉对应完成位
fn build_progress(state: &TransferState, staged_paths: &[PathBuf]) -> Progress {
    let mut files = Vec::with_capacity(state.files.len());
    let mut done_bytes = 0u64;
    for (index, file) in state.files.iter().enumerate() {
        let chunks = file.chunk_count(state.chunk_bytes);
        let bytes = resume::bitmap_bytes(chunks);
        let mut bitmap =
            resume::decode_bitmap(&file.bitmap, bytes).unwrap_or_else(|_| vec![0u8; bytes]);
        let on_disk = staged_paths
            .get(index)
            .and_then(|path| std::fs::metadata(path).ok())
            .map_or(0, |meta| meta.len());
        for chunk in 0..chunks {
            let Some((offset, len)) = file.span_at(chunk, state.chunk_bytes) else {
                continue;
            };
            if offset.saturating_add(len) > on_disk {
                resume::bit_clear(&mut bitmap, chunk);
            }
            if resume::bit_get(&bitmap, chunk) {
                done_bytes += len;
            }
        }
        files.push(FileProgress { bitmap, chunks });
    }
    Progress {
        files,
        done_bytes,
        received_bytes: 0,
    }
}

/// 建待拉取分块队列，跳已完成分块
///
/// # Errors
///
/// 文件数或分块长度超出协议上限时返回 [`Error::Protocol`]
fn build_queue(state: &TransferState, progress: &Progress) -> Result<VecDeque<ChunkRef>> {
    let mut queue = VecDeque::new();
    for (index, file) in state.files.iter().enumerate() {
        let Some(live) = progress.files.get(index) else {
            continue;
        };
        let file_index =
            u32::try_from(index).map_err(|_| Error::Protocol("文件数量超出协议上限".to_owned()))?;
        for chunk in 0..live.chunks {
            if resume::bit_get(&live.bitmap, chunk) {
                continue;
            }
            let Some((offset, len)) = file.span_at(chunk, state.chunk_bytes) else {
                continue;
            };
            let len = u32::try_from(len)
                .map_err(|_| Error::Protocol("分块长度超出协议上限".to_owned()))?;
            queue.push_back(ChunkRef {
                file: file_index,
                chunk_index: chunk,
                offset,
                len,
            });
        }
    }
    Ok(queue)
}

/// 命中的分块直接从旧副本拷到暂存文件，不占用网络
///
/// # Errors
///
/// 旧副本读取或暂存文件写入失败时返回 [`Error::Io`]
fn copy_matched_chunks(
    options: &RecvOptions,
    base: &TransferState,
    plans: &[Option<DiffPlan>],
    staged_paths: &[PathBuf],
    progress_cell: &Mutex<Progress>,
) -> Result<()> {
    let Some(old_root) = options.old_root.as_ref() else {
        return Ok(());
    };
    for (index, plan) in plans.iter().enumerate() {
        let Some(plan) = plan.as_ref() else {
            continue;
        };
        let Some(staged) = staged_paths.get(index) else {
            continue;
        };
        let Some(file_state) = base.files.get(index) else {
            continue;
        };
        let relative = sanitize_relative(&file_state.path)?;
        let old_path = join_within(old_root, &relative)?;
        if !old_path.is_file() {
            continue;
        }
        let mut source = std::fs::File::open(&old_path)
            .map_err(|err| Error::Io(format!("打开 {} 失败: {err}", old_path.display())))?;
        let file_index = u32::try_from(index).unwrap_or(u32::MAX);
        for (chunk_index, entry) in plan.plan.iter().enumerate() {
            let Some(matched) = entry.matched else {
                continue;
            };
            let Some(local) = usize::try_from(matched)
                .ok()
                .and_then(|slot| plan.local.get(slot))
            else {
                continue;
            };
            let chunk_index = u64::try_from(chunk_index).unwrap_or(u64::MAX);
            if is_chunk_done(progress_cell, index, chunk_index) {
                continue;
            }
            let len = usize::try_from(entry.span.len).unwrap_or(0);
            let mut buffer = vec![0u8; len];
            source
                .seek(SeekFrom::Start(local.span.offset))
                .map_err(io_error)?;
            source.read_exact(&mut buffer).map_err(io_error)?;
            write_at(staged, entry.span.offset, &buffer)?;
            record_done(
                progress_cell,
                &ChunkRef {
                    file: file_index,
                    chunk_index,
                    offset: entry.span.offset,
                    len: entry.span.len,
                },
                false,
            );
        }
    }
    Ok(())
}

/// 某块是否已完成
fn is_chunk_done(progress_cell: &Mutex<Progress>, file: usize, chunk: u64) -> bool {
    let guard = lock(progress_cell);
    guard
        .files
        .get(file)
        .is_some_and(|live| resume::bit_get(&live.bitmap, chunk))
}

/// 把一段数据写到指定偏移
fn write_at(path: &Path, offset: u64, data: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(io_error)?;
    file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
    file.write_all(data).map_err(io_error)
}

/// 由实时进度导出可落盘的续传状态
fn snapshot_state(base: &TransferState, progress_cell: &Mutex<Progress>) -> TransferState {
    let guard = lock(progress_cell);
    let files = base
        .files
        .iter()
        .zip(guard.files.iter())
        .map(|(declared, live)| FileState {
            path: declared.path.clone(),
            size: declared.size,
            bitmap: resume::encode_bitmap(&live.bitmap),
            spans: declared.spans.clone(),
        })
        .collect();
    TransferState {
        version: base.version,
        chunk_bytes: base.chunk_bytes,
        title: base.title.clone(),
        files,
    }
}

/// 组装进度快照
fn make_stats(progress: &Progress, total: u64, elapsed: Duration) -> TransferStats {
    let seconds = elapsed.as_secs_f64().max(0.001);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let rate = (progress.received_bytes as f64 / seconds) as u64;
    TransferStats {
        bytes_done: progress.done_bytes.min(total),
        bytes_total: total,
        bytes_per_sec: rate,
    }
}

/// 逐个核对暂存文件长度
fn verify_staged(staging: &Path, files: &[FileEntry]) -> Result<()> {
    for entry in files {
        let relative = sanitize_relative(&entry.path)?;
        let path = join_within(staging, &relative)?;
        let actual = std::fs::metadata(&path).map_or(0, |meta| meta.len());
        if actual != entry.size {
            return Err(Error::Checksum(format!(
                "{} 落盘长度 {actual} 与清单 {} 不符",
                entry.path, entry.size
            )));
        }
    }
    Ok(())
}

/// 原子写入认领文件
fn write_claim_files(options: &RecvOptions, claims: &[ClaimFile]) -> Result<Vec<PathBuf>> {
    let Some(root) = options.claim_root.as_ref() else {
        return Ok(Vec::new());
    };
    let mut written = Vec::with_capacity(claims.len());
    for claim in claims {
        let relative = sanitize_relative(&claim.relative_path)?;
        let path = join_within(root, &relative)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io_error)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &claim.content).map_err(io_error)?;
        if std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&path);
            std::fs::rename(&tmp, &path).map_err(io_error)?;
        }
        written.push(path);
    }
    Ok(written)
}

/// 建立控制连接并完成握手，返回连接与清单
fn open_control(
    peer: SocketAddr,
    pairing: Option<&str>,
    want: &str,
) -> Result<(Connection, RemoteManifest)> {
    let stream = connect(peer)?;
    let mut conn = Connection::new(stream)?;
    conn.send(&Message::Hello {
        version: PROTOCOL_VERSION,
        pairing: pairing.map(str::to_owned),
        want: want.to_owned(),
        need_manifest: true,
    })?;
    match conn.read()? {
        Some(Message::Manifest {
            title,
            platform: _,
            root_name,
            files,
            total_bytes,
            claim_files,
        }) => Ok((
            conn,
            RemoteManifest {
                title,
                root_name,
                files,
                total_bytes,
                claim_files,
            },
        )),
        Some(Message::Rejected { kind, message }) => Err(reject_error(kind, &message)),
        Some(other) => Err(Error::Protocol(format!("源端返回了意外消息: {other:?}"))),
        None => Err(Error::Protocol("源端在握手后立即关闭连接".to_owned())),
    }
}

/// 对端清单摘要，供界面在传输前做预检
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteInfo {
    /// 展示标题
    pub title: String,
    /// 目标根目录名
    pub root_name: String,
    /// 内容总字节数
    pub total_bytes: u64,
    /// 文件数量
    pub file_count: usize,
}

/// 只取对端清单，不传输数据
///
/// # Errors
///
/// 连接失败、被配对码拒绝或清单非法时返回错误
pub fn probe(peer: SocketAddr, pairing: Option<&str>, want: &str) -> Result<RemoteInfo> {
    let (_conn, manifest) = open_control(peer, pairing, want)?;
    Ok(RemoteInfo {
        title: manifest.title,
        root_name: manifest.root_name,
        total_bytes: manifest.total_bytes,
        file_count: manifest.files.len(),
    })
}

/// 差异预交换：把旧副本的分块摘要发给源端，换回缺失块清单
///
/// # Errors
///
/// 未给出旧副本根目录时返回全 `None`；交换失败返回协议错误
fn plan_diff(
    control: &mut Connection,
    options: &RecvOptions,
    manifest: &RemoteManifest,
) -> Result<Vec<Option<DiffPlan>>> {
    let Some(old_root) = options.old_root.as_ref() else {
        return Ok(vec![None; manifest.files.len()]);
    };
    let mut plans = Vec::with_capacity(manifest.files.len());
    for (index, entry) in manifest.files.iter().enumerate() {
        let relative = sanitize_relative(&entry.path)?;
        let old_path = join_within(old_root, &relative)?;
        if !old_path.is_file() {
            plans.push(None);
            continue;
        }
        let local = diff::index_file(&old_path)?;
        if local.is_empty() {
            plans.push(None);
            continue;
        }
        let file =
            u32::try_from(index).map_err(|_| Error::Protocol("文件数量超出协议上限".to_owned()))?;
        let hashes: Vec<[u8; DIGEST_BYTES]> = local.iter().map(|chunk| chunk.hash).collect();
        control.send_frame(&Frame {
            kind: KIND_HASHES,
            payload: diff::encode_hashes(file, &hashes),
        })?;
        let Some(frame) = read_frame(&mut control.reader)? else {
            return Err(Error::Protocol("源端在差异交换中关闭连接".to_owned()));
        };
        if frame.kind == crate::net::frame::KIND_CONTROL {
            return match protocol::decode(&frame.payload)? {
                Message::Rejected { kind, message } => Err(reject_error(kind, &message)),
                other => Err(Error::Protocol(format!("差异交换得到意外消息: {other:?}"))),
            };
        }
        if frame.kind != KIND_PLAN {
            return Err(Error::Protocol(format!(
                "差异交换得到意外的帧类型 {}",
                frame.kind
            )));
        }
        let (_, plan) = diff::decode_plan(&frame.payload)?;
        plans.push(Some(DiffPlan { plan, local }));
    }
    Ok(plans)
}

/// 建立带超时的连接
fn connect(peer: SocketAddr) -> Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&peer, CONNECT_TIMEOUT)
        .map_err(|err| Error::Io(format!("连接 {peer} 失败: {err}")))?;
    stream
        .set_nodelay(true)
        .map_err(|err| Error::Io(format!("设置 TCP 参数失败: {err}")))?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|err| Error::Io(format!("设置读取超时失败: {err}")))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|err| Error::Io(format!("设置写入超时失败: {err}")))?;
    Ok(stream)
}

/// 一条连接的读写封装，缓冲器在该连接生命周期内复用
struct Connection {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

impl Connection {
    /// 包装一条已建立的连接
    fn new(stream: TcpStream) -> Result<Self> {
        let reader_stream = stream
            .try_clone()
            .map_err(|err| Error::Io(format!("复制连接失败: {err}")))?;
        Ok(Self {
            reader: BufReader::new(reader_stream),
            writer: BufWriter::new(stream),
        })
    }

    /// 发送控制消息
    fn send(&mut self, message: &Message) -> Result<()> {
        write_frame(
            &mut self.writer,
            &Frame::control(protocol::encode(message)?),
        )
    }

    /// 发送任意类型的帧
    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        write_frame(&mut self.writer, frame)
    }

    /// 读取控制消息，对端关闭返回 `None`
    fn read(&mut self) -> Result<Option<Message>> {
        let Some(frame) = read_frame(&mut self.reader)? else {
            return Ok(None);
        };
        if !frame.is_control() {
            return Err(Error::Protocol("期望控制帧".to_owned()));
        }
        protocol::decode(&frame.payload).map(Some)
    }

    /// 请求一个分块并校验摘要
    fn fetch(&mut self, chunk: &ChunkRef) -> Result<Vec<u8>> {
        self.send(&Message::RequestChunk {
            file: chunk.file,
            offset: chunk.offset,
            len: chunk.len,
        })?;
        let Some(frame) = read_frame(&mut self.reader)? else {
            return Err(Error::Protocol("源端提前关闭连接".to_owned()));
        };
        if frame.is_control() {
            return match protocol::decode(&frame.payload)? {
                Message::Rejected { kind, message } => Err(reject_error(kind, &message)),
                other => Err(Error::Protocol(format!("分块请求得到意外消息: {other:?}"))),
            };
        }
        if frame.payload.len() < DIGEST_BYTES {
            return Err(Error::Checksum("分块帧缺少摘要".to_owned()));
        }
        let (digest, data) = frame.payload.split_at(DIGEST_BYTES);
        if data.len() != usize::try_from(chunk.len).unwrap_or(usize::MAX) {
            return Err(Error::Checksum(format!(
                "分块 {}/{} 长度 {} 与请求 {} 不符",
                chunk.file,
                chunk.offset,
                data.len(),
                chunk.len
            )));
        }
        if blake3::hash(data).as_bytes() != digest {
            return Err(Error::Checksum(format!(
                "分块 {}/{} 摘要不符",
                chunk.file, chunk.offset
            )));
        }
        Ok(data.to_vec())
    }
}

/// 工作线程：不断取分块、拉取、写盘，失败时重试
fn worker_loop(context: &Arc<WorkerContext>) -> Result<()> {
    let mut conn: Option<Connection> = None;
    while !context.stopped() {
        let Some(chunk) = context.next_chunk() else {
            break;
        };
        let mut last_error: Option<Error> = None;
        let mut done = false;
        for _ in 0..MAX_ATTEMPTS {
            if context.stopped() {
                return Ok(());
            }
            if conn.is_none() {
                conn = open_worker(context).ok();
            }
            let Some(active) = conn.as_mut() else {
                thread::sleep(RETRY_DELAY);
                last_error = Some(Error::Io("连接源端失败".to_owned()));
                continue;
            };
            match active.fetch(&chunk) {
                Ok(data) => {
                    write_chunk(context, &chunk, &data)?;
                    record_done(&context.progress, &chunk, true);
                    done = true;
                    break;
                }
                Err(err) => {
                    let checksum = matches!(err, Error::Checksum(_));
                    last_error = Some(err);
                    if !checksum {
                        conn = None;
                    }
                }
            }
        }
        if !done {
            context.abort.store(true, Ordering::Relaxed);
            return Err(last_error.unwrap_or_else(|| Error::Io("分块重试耗尽".to_owned())));
        }
    }
    if let Some(active) = conn.as_mut() {
        let _ = active.send(&Message::Bye);
    }
    Ok(())
}

/// 为工作线程建立连接并完成握手
fn open_worker(context: &Arc<WorkerContext>) -> Result<Connection> {
    let stream = connect(context.peer)?;
    let mut conn = Connection::new(stream)?;
    conn.send(&Message::Hello {
        version: PROTOCOL_VERSION,
        pairing: context.pairing.clone(),
        want: context.want.clone(),
        need_manifest: false,
    })?;
    match conn.read()? {
        Some(Message::Ready) => Ok(conn),
        Some(Message::Rejected { kind, message }) => Err(reject_error(kind, &message)),
        Some(other) => Err(Error::Protocol(format!("握手收到意外消息: {other:?}"))),
        None => Err(Error::Protocol("源端在握手后立即关闭连接".to_owned())),
    }
}

/// 把分块写到暂存文件的指定偏移
fn write_chunk(context: &Arc<WorkerContext>, chunk: &ChunkRef, data: &[u8]) -> Result<()> {
    let Some(path) = usize::try_from(chunk.file)
        .ok()
        .and_then(|index| context.staged_paths.get(index))
    else {
        return Err(Error::Protocol("分块指向未知文件".to_owned()));
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(io_error)?;
    file.seek(SeekFrom::Start(chunk.offset)).map_err(io_error)?;
    file.write_all(data).map_err(io_error)?;
    Ok(())
}

/// 记录一个分块完成，`from_network` 区分远程拉取与本地命中拷贝
fn record_done(progress_cell: &Mutex<Progress>, chunk: &ChunkRef, from_network: bool) {
    let mut guard = lock(progress_cell);
    let Some(index) = usize::try_from(chunk.file).ok() else {
        return;
    };
    let mut newly = false;
    if let Some(file) = guard.files.get_mut(index) {
        if !resume::bit_get(&file.bitmap, chunk.chunk_index) {
            resume::bit_set(&mut file.bitmap, chunk.chunk_index);
            newly = true;
        }
    }
    if newly {
        guard.done_bytes += u64::from(chunk.len);
        if from_network {
            guard.received_bytes += u64::from(chunk.len);
        }
    }
}

/// 把拒绝原因转成 crate 错误
fn reject_error(kind: RejectKind, message: &str) -> Error {
    match kind {
        RejectKind::Pairing => Error::PairingRejected,
        _ => Error::Protocol(message.to_owned()),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn sample_manifest(files: Vec<(&str, u64)>) -> RemoteManifest {
        RemoteManifest {
            title: "示例".to_owned(),
            root_name: "example".to_owned(),
            files: files
                .into_iter()
                .map(|(path, size)| FileEntry {
                    path: path.to_owned(),
                    size,
                })
                .collect(),
            total_bytes: 0,
            claim_files: Vec::new(),
        }
    }

    #[test]
    fn state_matches_only_for_same_manifest() {
        let manifest = sample_manifest(vec![("a.bin", 10)]);
        let plans = vec![None];
        let state = initial_state(&manifest, 4, &plans);
        assert!(state_matches(&state, &manifest, 4, &plans));
        assert!(!state_matches(&state, &manifest, 8, &plans));
        assert!(!state_matches(
            &state,
            &sample_manifest(vec![("a.bin", 11)]),
            4,
            &plans
        ));
        assert!(!state_matches(
            &state,
            &sample_manifest(vec![("b.bin", 10)]),
            4,
            &plans
        ));
    }

    #[test]
    fn diff_plan_replaces_chunk_boundaries() {
        let manifest = sample_manifest(vec![("a.bin", 10)]);
        let plan = DiffPlan {
            plan: vec![
                diff::PlanEntry {
                    span: diff::ChunkSpan { offset: 0, len: 6 },
                    matched: Some(0),
                },
                diff::PlanEntry {
                    span: diff::ChunkSpan { offset: 6, len: 4 },
                    matched: None,
                },
            ],
            local: Vec::new(),
        };
        let state = initial_state(&manifest, 4, &[Some(plan.clone())]);
        assert_eq!(state.files[0].spans.len(), 2);
        assert_eq!(state.files[0].chunk_count(4), 2);
        assert!(state_matches(&state, &manifest, 4, &[Some(plan)]));
        assert!(!state_matches(&state, &manifest, 4, &[None]));
    }

    #[test]
    fn queue_skips_completed_chunks() {
        let manifest = sample_manifest(vec![("a.bin", 10)]);
        let plans = vec![None];
        let mut state = initial_state(&manifest, 4, &plans);
        let mut bitmap = resume::decode_bitmap(&state.files[0].bitmap, 1).expect("decode");
        resume::bit_set(&mut bitmap, 0);
        state.files[0].bitmap = resume::encode_bitmap(&bitmap);
        let dir = std::env::temp_dir().join(format!("gamelift-queue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let paths = prepare_staging(&dir, &manifest.files).expect("staging");
        std::fs::write(&paths[0], vec![0u8; 4]).expect("write");
        let progress = build_progress(&state, &paths);
        let queue = build_queue(&state, &progress).expect("queue");
        let chunks: Vec<u64> = queue.iter().map(|chunk| chunk.chunk_index).collect();
        assert_eq!(chunks, vec![1, 2]);
        assert_eq!(progress.done_bytes, 4);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn progress_drops_bits_beyond_disk_length() {
        let manifest = sample_manifest(vec![("a.bin", 10)]);
        let plans = vec![None];
        let mut state = initial_state(&manifest, 4, &plans);
        let mut bitmap = resume::decode_bitmap(&state.files[0].bitmap, 1).expect("decode");
        resume::bit_set(&mut bitmap, 0);
        resume::bit_set(&mut bitmap, 1);
        resume::bit_set(&mut bitmap, 2);
        state.files[0].bitmap = resume::encode_bitmap(&bitmap);
        let dir = std::env::temp_dir().join(format!("gamelift-trunc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let paths = prepare_staging(&dir, &manifest.files).expect("staging");
        std::fs::write(&paths[0], vec![0u8; 5]).expect("write");
        let progress = build_progress(&state, &paths);
        let queue = build_queue(&state, &progress).expect("queue");
        let chunks: Vec<u64> = queue.iter().map(|chunk| chunk.chunk_index).collect();
        assert_eq!(chunks, vec![1, 2]);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn stats_report_rate_and_progress() {
        let progress = Progress {
            files: Vec::new(),
            done_bytes: 50,
            received_bytes: 100,
        };
        let stats = make_stats(&progress, 200, Duration::from_secs(2));
        assert_eq!(stats.bytes_done, 50);
        assert_eq!(stats.bytes_total, 200);
        assert_eq!(stats.bytes_per_sec, 50);
    }

    #[test]
    fn reject_error_maps_pairing() {
        assert!(matches!(
            reject_error(RejectKind::Pairing, "x"),
            Error::PairingRejected
        ));
        assert!(matches!(
            reject_error(RejectKind::Version, "x"),
            Error::Protocol(_)
        ));
    }
}
