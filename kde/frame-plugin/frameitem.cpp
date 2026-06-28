#include "frameitem.h"
#include "gpu_renderer.h"

#include <QLoggingCategory>
#include <QQuickWindow>
#include <QOpenGLContext>
#include <QAudioFormat>
#include <atomic>
#include <cstring>

Q_LOGGING_CATEGORY(logFrames, "better-wallpaper.plasma.frames")

namespace {
constexpr qsizetype HeaderSize = 64;
constexpr qsizetype SequenceOffset = 32;
constexpr qsizetype ActiveSlotOffset = 40;
constexpr qsizetype PtsOffset = 44;
constexpr qsizetype PtsDenOffset = 52;

constexpr qsizetype AudioHeaderSize = 64;
constexpr qsizetype AudioWritePosOff = 16;
constexpr qsizetype AudioReadPosOff = 24;

template<typename T> T readValue(const uchar *data, qsizetype offset) {
    T value;
    std::memcpy(&value, data + offset, sizeof(value));
    return value;
}

void *qtGlLoader(const char *name) {
    auto *ctx = QOpenGLContext::currentContext();
    if (!ctx) return nullptr;
    return reinterpret_cast<void *>(
        ctx->getProcAddress(QByteArray(name)));
}
} // anonymous namespace

/* ---- FrameRenderNode -------------------------------------------------- */

FrameRenderNode::FrameRenderNode(GpuRenderer *renderer, const QRectF &rect,
                                 int fillMode, uint32_t outW, uint32_t outH)
    : m_renderer(renderer), m_rect(rect), m_fillMode(fillMode),
      m_outputW(outW), m_outputH(outH) {}

QSGRenderNode::StateFlags FrameRenderNode::changedStates() const {
    return BlendState | ViewportState;
}

void FrameRenderNode::render(const RenderState *) {
    if (!m_renderer) return;
    gpu_renderer_draw(m_renderer, m_outputW, m_outputH, m_fillMode);
}

QSGRenderNode::RenderingFlags FrameRenderNode::flags() const {
    return BoundedRectRendering;
}

QRectF FrameRenderNode::rect() const { return m_rect; }

void FrameRenderNode::releaseResources() {}

void FrameRenderNode::setFillMode(int mode) { m_fillMode = mode; }
void FrameRenderNode::setOutputSize(uint32_t w, uint32_t h) {
    m_outputW = w; m_outputH = h;
}
void FrameRenderNode::setRect(const QRectF &rect) { m_rect = rect; }

/* ---- FrameItem -------------------------------------------------------- */

FrameItem::FrameItem(QQuickItem *parent) : QQuickItem(parent) {
    setFlag(ItemHasContents, true);
    m_reconnectTimer.setInterval(250);
    connect(&m_reconnectTimer, &QTimer::timeout,
            this, &FrameItem::connectNotificationSocket);
    connect(&m_notificationSocket, &QLocalSocket::connected, this, [this] {
        m_reconnectTimer.stop();
        qCInfo(logFrames) << "notification socket connected";
        update();
    });
    connect(&m_notificationSocket, &QLocalSocket::readyRead,
            this, &FrameItem::consumeNotifications);
    connect(&m_notificationSocket, &QLocalSocket::disconnected, this, [this] {
        if (!m_source.isEmpty()) m_reconnectTimer.start();
    });
    connect(&m_notificationSocket, &QLocalSocket::errorOccurred, this,
            [this](QLocalSocket::LocalSocketError) {
                if (!m_source.isEmpty() && !m_reconnectTimer.isActive())
                    m_reconnectTimer.start();
            });
    m_statsTimer.start();
}

FrameItem::~FrameItem() {
    closeAudio();
    if (m_renderer) gpu_renderer_destroy(m_renderer);
    unmap();
}

void FrameItem::setSource(const QString &source) {
    if (source == m_source) return;
    unmap();
    closeAudio();
    m_source = source;
    m_lastSequence = 0;
    if (m_renderer) {
        gpu_renderer_destroy(m_renderer);
        m_renderer = nullptr;
    }
    qCInfo(logFrames) << "shared frame source changed" << source;
    emit sourceChanged();
    m_notificationSocket.abort();
    m_reconnectTimer.stop();
    if (!m_source.isEmpty()) {
        connectNotificationSocket();
        ensureAudio();  // open PCM ring buffer
    }
    update();
}

void FrameItem::connectNotificationSocket() {
    if (m_source.isEmpty()
        || m_notificationSocket.state() != QLocalSocket::UnconnectedState)
        return;
    m_notificationSocket.connectToServer(
        m_source + QLatin1String(".notify"), QIODevice::ReadOnly);
}

void FrameItem::consumeNotifications() {
    m_notificationSocket.readAll();
    update();
}

void FrameItem::setFillMode(const QString &fillMode) {
    if (fillMode == m_fillMode) return;
    m_fillMode = fillMode;
    emit fillModeChanged();
    update();
}

int FrameItem::fillModeInt() const {
    if (m_fillMode == QLatin1String("stretch")) return 2;
    if (m_fillMode == QLatin1String("contain")) return 1;
    return 0;
}

bool FrameItem::ensureMapped() {
    if (m_mapping) return true;
    if (m_source.isEmpty()) return false;
    m_file.setFileName(m_source);
    if (!m_file.open(QIODevice::ReadOnly) || m_file.size() < HeaderSize)
        return false;
    m_mappingSize = m_file.size();
    m_mapping = m_file.map(0, m_mappingSize);
    if (!m_mapping || std::memcmp(m_mapping, "BWFRAME1", 8) != 0) {
        qCWarning(logFrames) << "invalid shared frame buffer" << m_source;
        unmap();
        return false;
    }
    qCInfo(logFrames) << "shared frame buffer mapped" << m_source << m_mappingSize;
    return true;
}

void FrameItem::unmap() {
    if (m_mapping) m_file.unmap(m_mapping);
    m_mapping = nullptr;
    m_mappingSize = 0;
    if (m_file.isOpen()) m_file.close();
}

/* ---- Audio PCM ring buffer ------------------------------------------- */

bool FrameItem::ensureAudio() {
    if (m_audioSink) return true;
    if (m_source.isEmpty()) return false;

    const QString audioPath = m_source + QLatin1String(".audio");
    m_audioFile.setFileName(audioPath);
    if (!m_audioFile.open(QIODevice::ReadOnly) || m_audioFile.size() < AudioHeaderSize)
        return false;

    m_audioSize = m_audioFile.size();
    m_audioMapping = m_audioFile.map(0, m_audioSize);
    if (!m_audioMapping || std::memcmp(m_audioMapping, "BWAD", 4) != 0) {
        qCWarning(logFrames) << "invalid audio ring buffer" << audioPath;
        m_audioFile.unmap(m_audioMapping);
        m_audioMapping = nullptr;
        m_audioFile.close();
        return false;
    }

    m_audioRate = readValue<uint32_t>(m_audioMapping, 4);
    m_audioChannels = readValue<uint32_t>(m_audioMapping, 8);
    m_audioBufferSamples = readValue<uint32_t>(m_audioMapping, 12);

    QAudioFormat fmt;
    fmt.setSampleRate(int(m_audioRate));
    fmt.setChannelCount(int(m_audioChannels));
    fmt.setSampleFormat(QAudioFormat::Float);

    m_audioSink = new QAudioSink(fmt, this);
    m_audioDevice = m_audioSink->start();

    qCInfo(logFrames) << "audio PCM ring buffer opened"
                      << audioPath
                      << "rate" << m_audioRate
                      << "ch" << m_audioChannels
                      << "buf_samples" << m_audioBufferSamples;
    return true;
}

void FrameItem::feedAudio() {
    if (!m_audioMapping || !m_audioDevice) return;

    const auto writePos = std::atomic_ref<quint64>(
        *reinterpret_cast<quint64 *>(m_audioMapping + AudioWritePosOff))
        .load(std::memory_order_acquire);

    auto &readPos = *reinterpret_cast<quint64 *>(m_audioMapping + AudioReadPosOff);

    if (writePos <= readPos) return;

    const quint64 avail = writePos - readPos;
    const quint64 offset = readPos % m_audioBufferSamples;
    const quint64 dataOff = AudioHeaderSize + offset * m_audioChannels * sizeof(float);
    const quint64 bytesAvail = avail * m_audioChannels * sizeof(float);

    // Simple: read at most 4096 samples per call
    const quint64 toRead = qMin(avail, quint64(4096));
    const quint64 toReadBytes = toRead * m_audioChannels * sizeof(float);

    // Handle ring buffer wrap
    const quint64 firstChunk = qMin(toReadBytes, quint64(m_audioSize) - dataOff);
    m_audioDevice->write(
        reinterpret_cast<const char *>(m_audioMapping + dataOff),
        qint64(firstChunk));

    if (firstChunk < toReadBytes) {
        m_audioDevice->write(
            reinterpret_cast<const char *>(m_audioMapping + AudioHeaderSize),
            qint64(toReadBytes - firstChunk));
    }

    readPos += toRead;
}

void FrameItem::closeAudio() {
    if (m_audioSink) {
        m_audioSink->stop();
        delete m_audioSink;
        m_audioSink = nullptr;
        m_audioDevice = nullptr;
    }
    if (m_audioMapping) {
        m_audioFile.unmap(m_audioMapping);
        m_audioMapping = nullptr;
    }
    m_audioSize = 0;
    if (m_audioFile.isOpen()) m_audioFile.close();
}

/* ---- Render ---------------------------------------------------------- */

QSGNode *FrameItem::updatePaintNode(QSGNode *oldNode,
                                    UpdatePaintNodeData *) {
    auto *node = static_cast<FrameRenderNode *>(oldNode);

    // Resolve output pixel size
    const qreal dpr = window() ? window()->effectiveDevicePixelRatio() : 1.0;
    const QRectF bounds = boundingRect();
    const uint32_t outW = static_cast<uint32_t>(qMax(bounds.width() * dpr, 1.0));
    const uint32_t outH = static_cast<uint32_t>(qMax(bounds.height() * dpr, 1.0));

    // Lazy-init GPU renderer
    if (!m_renderer && window()) {
        m_renderer = gpu_renderer_create(qtGlLoader, outW, outH);
        if (!m_renderer) {
            qCWarning(logFrames) << "failed to create GPU renderer";
            return node;
        }
        qCInfo(logFrames) << "GPU renderer initialised, output" << outW << "x" << outH;
    }
    if (!m_renderer) return node;

    // Feed audio samples
    feedAudio();

    // Read latest frame from shared memory
    if (!ensureMapped()) return node;

    const auto sequence =
        std::atomic_ref<quint64>(
            *reinterpret_cast<quint64 *>(m_mapping + SequenceOffset))
            .load(std::memory_order_acquire);

    if (sequence == 0 || sequence == m_lastSequence) {
        if (!node) {
            node = new FrameRenderNode(m_renderer, bounds, fillModeInt(), outW, outH);
        } else {
            node->setFillMode(fillModeInt());
            node->setOutputSize(outW, outH);
            node->setRect(bounds);
        }
        return node;
    }

    if (m_lastSequence != 0 && sequence > m_lastSequence + 1)
        m_skippedFrames += sequence - m_lastSequence - 1;

    const quint32 width = readValue<quint32>(m_mapping, 8);
    const quint32 height = readValue<quint32>(m_mapping, 12);
    const quint32 stride = readValue<quint32>(m_mapping, 16);
    const quint32 slotCount = readValue<quint32>(m_mapping, 20);
    const quint64 frameBytes = readValue<quint64>(m_mapping, 24);
    const quint32 slot =
        std::atomic_ref<quint32>(
            *reinterpret_cast<quint32 *>(m_mapping + ActiveSlotOffset))
            .load(std::memory_order_relaxed);
    const quint64 offset = HeaderSize + quint64(slot) * frameBytes;

    if (!width || !height || slot >= slotCount
        || offset + frameBytes > quint64(m_mappingSize)) {
        qCWarning(logFrames) << "invalid frame slot" << slot << "for seq" << sequence;
        return node;
    }

    // Upload frame via PBO
    QElapsedTimer uploadTimer;
    uploadTimer.start();

    const uchar *frameData = m_mapping + offset;
    if (!gpu_renderer_upload(m_renderer, frameData,
                             static_cast<uint32_t>(frameBytes),
                             width, height, stride)) {
        qCWarning(logFrames) << "GPU upload failed for seq" << sequence;
        return node;
    }

    m_uploadTimeNs += uploadTimer.nsecsElapsed();
    m_lastSequence = sequence;
    ++m_renderedFrames;

    if (!node) {
        node = new FrameRenderNode(m_renderer, bounds, fillModeInt(), outW, outH);
    } else {
        node->setFillMode(fillModeInt());
        node->setOutputSize(outW, outH);
        node->setRect(bounds);
    }
    node->markDirty(QSGNode::DirtyMaterial);

    if (m_statsTimer.elapsed() >= 1000) {
        qint64 audioAvail = 0;
        if (m_audioMapping) {
            const auto wPos = std::atomic_ref<quint64>(
                *reinterpret_cast<quint64 *>(m_audioMapping + AudioWritePosOff))
                .load(std::memory_order_acquire);
            const auto rPos = *reinterpret_cast<quint64 *>(m_audioMapping + AudioReadPosOff);
            audioAvail = qint64(wPos - rPos);
        }
        qCInfo(logFrames) << "render-perf"
                          << "rendered" << m_renderedFrames
                          << "skipped" << m_skippedFrames
                          << "upload_avg_us"
                          << double(m_uploadTimeNs) / double(m_renderedFrames) / 1000.0
                          << "audio_buf" << audioAvail;
        m_renderedFrames = 0;
        m_skippedFrames = 0;
        m_uploadTimeNs = 0;
        m_statsTimer.restart();
    }

    return node;
}

