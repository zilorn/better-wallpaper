#include "frameitem.h"
#include "gpu_renderer.h"

#include <QLoggingCategory>
#include <QMutexLocker>
#include <QOpenGLContext>
#include <QQuickWindow>
#include <QMetaObject>
#include <chrono>

Q_LOGGING_CATEGORY(logFrames, "better-wallpaper.plasma.frames")

namespace {
void *qtGlLoader(const char *name) {
    auto *ctx = QOpenGLContext::currentContext();
    return ctx ? reinterpret_cast<void *>(ctx->getProcAddress(QByteArray(name))) : nullptr;
}
}

FrameRenderNode::FrameRenderNode(GpuRenderer *renderer, const QRectF &rect,
                                 int fillMode, uint32_t outW, uint32_t outH)
    : m_renderer(renderer), m_rect(rect), m_fillMode(fillMode),
      m_outputW(outW), m_outputH(outH) {}
QSGRenderNode::StateFlags FrameRenderNode::changedStates() const { return BlendState | ViewportState; }
void FrameRenderNode::render(const RenderState *) {
    if (m_renderer) gpu_renderer_draw(m_renderer, m_outputW, m_outputH, m_fillMode);
}
QSGRenderNode::RenderingFlags FrameRenderNode::flags() const { return BoundedRectRendering; }
QRectF FrameRenderNode::rect() const { return m_rect; }
void FrameRenderNode::releaseResources() {}
void FrameRenderNode::setFillMode(int mode) { m_fillMode = mode; }
void FrameRenderNode::setOutputSize(uint32_t w, uint32_t h) { m_outputW = w; m_outputH = h; }
void FrameRenderNode::setRect(const QRectF &rect) { m_rect = rect; }

FrameItem::FrameItem(QQuickItem *parent) : QQuickItem(parent) {
    setFlag(ItemHasContents, true);
}

FrameItem::~FrameItem() {
    closeDecoder();
    if (m_renderer) gpu_renderer_destroy(m_renderer);
}

void FrameItem::setSource(const QString &source) {
    if (source == m_source) return;
    closeDecoder();
    m_source = source;
    emit sourceChanged();
    if (source.isEmpty()) return;
    startDecoder();
}

void FrameItem::setFillMode(const QString &mode) {
    if (mode == m_fillMode) return;
    m_fillMode = mode;
    emit fillModeChanged();
    update();
}

void FrameItem::setPaused(bool paused) {
    if (paused == m_paused) return;
    m_paused = paused;
    m_decodePaused.store(paused, std::memory_order_release);
    m_decodeWake.notify_all();
    emit pausedChanged();
    qCInfo(logFrames) << "direct video pause state changed" << paused;
}

void FrameItem::setLoopPlayback(bool enabled) {
    if (enabled == m_loopPlayback) return;
    m_loopPlayback = enabled;
    m_decodeLooping.store(enabled, std::memory_order_release);
    emit loopPlaybackChanged();
}

void FrameItem::setPlaybackPosition(qint64 position) {
    if (position == m_externalPositionMs) return;
    m_externalPositionMs = qMax<qint64>(position, 0);
    emit playbackPositionChanged();
}

void FrameItem::startDecoder() {
    if (m_source.isEmpty()) return;
    const quint64 generation = m_decodeGeneration.fetch_add(1) + 1;
    {
        std::lock_guard lock(m_decodeStateMutex);
        m_stopDecode = false;
    }
    m_decodePaused.store(m_paused, std::memory_order_release);
    m_decodeLooping.store(m_loopPlayback, std::memory_order_release);
    m_decodeThread = std::thread(&FrameItem::decodeLoop, this, m_source, generation);
}

void FrameItem::closeDecoder() {
    m_decodeGeneration.fetch_add(1);
    {
        std::lock_guard lock(m_decodeStateMutex);
        m_stopDecode = true;
    }
    m_decodeWake.notify_all();
    if (m_decodeThread.joinable()) m_decodeThread.join();
    m_deliveryQueued.store(false, std::memory_order_release);
    QMutexLocker lock(&m_frameMutex);
    m_frame.clear();
    ++m_frameSequence;
}

void FrameItem::decodeLoop(QString source, quint64 generation) {
    const QByteArray path = source.toUtf8();
    KdeVideoDecoder *decoder = kde_video_decoder_open(path.constData());
    if (!decoder) {
        qCWarning(logFrames) << "failed to open direct video source" << source;
        return;
    }
    qCInfo(logFrames) << "video decode worker started" << source;
    using Clock = std::chrono::steady_clock;
    auto playbackStart = Clock::now();
    quint64 firstPts = 0;
    bool firstFrame = true;
    quint64 decoded = 0;
    quint64 dropped = 0;

    while (generation == m_decodeGeneration.load(std::memory_order_acquire)) {
        {
            std::unique_lock lock(m_decodeStateMutex);
            if (m_stopDecode) break;
            if (m_decodePaused.load(std::memory_order_acquire)) {
                const auto pausedAt = Clock::now();
                m_decodeWake.wait(lock, [this] {
                    return m_stopDecode || !m_decodePaused.load(std::memory_order_acquire);
                });
                playbackStart += Clock::now() - pausedAt;
                if (m_stopDecode) break;
            }
        }

        KdeVideoFrame raw{};
        const int result = kde_video_decoder_next(decoder, &raw);
        if (result == 0 && m_decodeLooping.load(std::memory_order_acquire)
            && kde_video_decoder_seek_start(decoder)) {
            playbackStart = Clock::now();
            firstFrame = true;
            continue;
        }
        if (result <= 0) {
            if (result < 0) qCWarning(logFrames) << "video decode worker stopped with error" << result;
            break;
        }
        ++decoded;
        if (firstFrame) {
            firstPts = raw.pts_millis;
            playbackStart = Clock::now();
            firstFrame = false;
        }
        const quint64 relativePts = raw.pts_millis >= firstPts ? raw.pts_millis - firstPts : 0;
        const auto target = playbackStart + std::chrono::milliseconds(relativePts);
        const auto now = Clock::now();
        if (now > target + std::chrono::milliseconds(80)) {
            ++dropped;
            continue;
        }
        {
            std::unique_lock lock(m_decodeStateMutex);
            m_decodeWake.wait_until(lock, target, [this, generation] {
                return m_stopDecode || m_decodePaused.load(std::memory_order_acquire)
                    || generation != m_decodeGeneration.load(std::memory_order_acquire);
            });
            if (m_stopDecode || generation != m_decodeGeneration.load(std::memory_order_acquire)) break;
            if (m_decodePaused.load(std::memory_order_acquire)) continue;
        }
        QByteArray pixels(reinterpret_cast<const char *>(raw.data), qsizetype(raw.len));
        {
            QMutexLocker lock(&m_frameMutex);
            m_frame = std::move(pixels);
            m_frameWidth = raw.width;
            m_frameHeight = raw.height;
            m_frameStride = raw.stride;
            ++m_frameSequence;
        }
        if (!m_deliveryQueued.exchange(true, std::memory_order_acq_rel)) {
            QMetaObject::invokeMethod(this, [this, generation] { deliverFrame(generation); },
                                      Qt::QueuedConnection);
        }
    }
    kde_video_decoder_destroy(decoder);
    qCInfo(logFrames) << "video decode worker ended" << source
                      << "decoded" << decoded << "late_dropped" << dropped;
}

void FrameItem::deliverFrame(quint64 generation) {
    m_deliveryQueued.store(false, std::memory_order_release);
    if (generation != m_decodeGeneration.load(std::memory_order_acquire)) return;
    update();
}

int FrameItem::fillModeInt() const {
    if (m_fillMode == QLatin1String("stretch")) return 2;
    if (m_fillMode == QLatin1String("contain")) return 1;
    return 0;
}

QSGNode *FrameItem::updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *) {
    auto *node = static_cast<FrameRenderNode *>(oldNode);
    const qreal dpr = window() ? window()->effectiveDevicePixelRatio() : 1.0;
    const QRectF bounds = boundingRect();
    const uint32_t outW = uint32_t(qMax(bounds.width() * dpr, 1.0));
    const uint32_t outH = uint32_t(qMax(bounds.height() * dpr, 1.0));
    if (!m_renderer && window()) m_renderer = gpu_renderer_create(qtGlLoader, outW, outH);
    if (!m_renderer) return node;

    QByteArray frame;
    uint32_t width = 0, height = 0, stride = 0;
    quint64 sequence = 0;
    {
        QMutexLocker lock(&m_frameMutex);
        sequence = m_frameSequence;
        if (sequence != m_uploadedSequence) {
            frame = m_frame;
            width = m_frameWidth;
            height = m_frameHeight;
            stride = m_frameStride;
        }
    }
    if (!frame.isEmpty() && gpu_renderer_upload(
            m_renderer, reinterpret_cast<const uint8_t *>(frame.constData()),
            uint32_t(frame.size()), width, height, stride)) {
        m_uploadedSequence = sequence;
    }
    if (!node) node = new FrameRenderNode(m_renderer, bounds, fillModeInt(), outW, outH);
    node->setFillMode(fillModeInt());
    node->setOutputSize(outW, outH);
    node->setRect(bounds);
    node->markDirty(QSGNode::DirtyMaterial);
    return node;
}
