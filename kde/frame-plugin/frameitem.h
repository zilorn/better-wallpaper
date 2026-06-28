#pragma once

#include <QFile>
#include <QElapsedTimer>
#include <QQuickItem>
#include <QLocalSocket>
#include <QTimer>
#include <QSGRenderNode>
#include <QAudioSink>
#include <QIODevice>

struct GpuRenderer;

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

public:
    explicit FrameItem(QQuickItem *parent = nullptr);
    ~FrameItem() override;
    QString source() const { return m_source; }
    QString fillMode() const { return m_fillMode; }
    void setSource(const QString &source);
    void setFillMode(const QString &fillMode);

signals:
    void sourceChanged();
    void fillModeChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *) override;

private:
    bool ensureMapped();
    void connectNotificationSocket();
    void consumeNotifications();
    void unmap();
    int fillModeInt() const;

    // Audio ring buffer
    bool ensureAudio();
    void feedAudio();
    void closeAudio();

    QString m_source;
    QString m_fillMode = QStringLiteral("cover");

    // Video shared memory
    QFile m_file;
    uchar *m_mapping = nullptr;
    qsizetype m_mappingSize = 0;
    quint64 m_lastSequence = 0;
    GpuRenderer *m_renderer = nullptr;

    // Audio PCM ring buffer
    QFile m_audioFile;
    uchar *m_audioMapping = nullptr;
    qsizetype m_audioSize = 0;
    QAudioSink *m_audioSink = nullptr;
    QIODevice *m_audioDevice = nullptr;
    uint32_t m_audioRate = 0;
    uint32_t m_audioChannels = 0;
    uint32_t m_audioBufferSamples = 0;

    // Stats
    quint64 m_renderedFrames = 0;
    quint64 m_skippedFrames = 0;
    qint64 m_uploadTimeNs = 0;
    QElapsedTimer m_statsTimer;

    QLocalSocket m_notificationSocket;
    QTimer m_reconnectTimer;
};

