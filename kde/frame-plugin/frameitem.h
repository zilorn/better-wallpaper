#pragma once

#include <QByteArray>
#include <QMutex>
#include <QQuickItem>
#include <QSGRenderNode>
#include <atomic>
#include <condition_variable>
#include <mutex>
#include <thread>

struct GpuRenderer;
struct KdeVideoDecoder;

class FrameRenderNode : public QSGRenderNode {
public:
    FrameRenderNode(GpuRenderer *renderer, const QRectF &rect,
                    int fillMode, uint32_t outW, uint32_t outH);
    StateFlags changedStates() const override;
    void render(const RenderState *state) override;
    RenderingFlags flags() const override;
    QRectF rect() const override;
    void releaseResources() override;
    void setFillMode(int mode);
    void setOutputSize(uint32_t w, uint32_t h);
    void setRect(const QRectF &rect);

private:
    GpuRenderer *m_renderer;
    QRectF m_rect;
    int m_fillMode;
    uint32_t m_outputW = 0;
    uint32_t m_outputH = 0;
};

class FrameItem : public QQuickItem {
    Q_OBJECT
    Q_PROPERTY(QString source READ source WRITE setSource NOTIFY sourceChanged)
    Q_PROPERTY(QString fillMode READ fillMode WRITE setFillMode NOTIFY fillModeChanged)
    Q_PROPERTY(bool paused READ paused WRITE setPaused NOTIFY pausedChanged)
    Q_PROPERTY(bool loopPlayback READ loopPlayback WRITE setLoopPlayback NOTIFY loopPlaybackChanged)
    Q_PROPERTY(qint64 playbackPosition READ playbackPosition WRITE setPlaybackPosition NOTIFY playbackPositionChanged)

public:
    explicit FrameItem(QQuickItem *parent = nullptr);
    ~FrameItem() override;
    QString source() const { return m_source; }
    QString fillMode() const { return m_fillMode; }
    bool paused() const { return m_paused; }
    bool loopPlayback() const { return m_loopPlayback; }
    qint64 playbackPosition() const { return m_externalPositionMs; }
    void setSource(const QString &source);
    void setFillMode(const QString &fillMode);
    void setPaused(bool paused);
    void setLoopPlayback(bool enabled);
    void setPlaybackPosition(qint64 position);

signals:
    void sourceChanged();
    void fillModeChanged();
    void pausedChanged();
    void loopPlaybackChanged();
    void playbackPositionChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *) override;

private:
    void startDecoder();
    void closeDecoder();
    void decodeLoop(QString source, quint64 generation);
    void deliverFrame(quint64 generation);
    int fillModeInt() const;

    QString m_source;
    QString m_fillMode = QStringLiteral("cover");
    bool m_paused = false;
    bool m_loopPlayback = true;
    qint64 m_externalPositionMs = 0;

    std::thread m_decodeThread;
    std::mutex m_decodeStateMutex;
    std::condition_variable m_decodeWake;
    bool m_stopDecode = false;
    std::atomic_bool m_decodePaused{false};
    std::atomic_bool m_decodeLooping{true};
    std::atomic<quint64> m_decodeGeneration{0};
    std::atomic_bool m_deliveryQueued{false};

    mutable QMutex m_frameMutex;
    QByteArray m_frame;
    uint32_t m_frameWidth = 0;
    uint32_t m_frameHeight = 0;
    uint32_t m_frameStride = 0;
    quint64 m_frameSequence = 0;
    quint64 m_uploadedSequence = 0;
    GpuRenderer *m_renderer = nullptr;
};
