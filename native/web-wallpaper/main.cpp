// SPDX-License-Identifier: MIT
#include <QGuiApplication>
#include <QElapsedTimer>
#include <QJsonDocument>
#include <QJsonArray>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickView>
#include <QScreen>
#include <QTimer>
#include <QtWebEngineQuick/qtwebenginequickglobal.h>
#include <LayerShellQt/Window>
#include <fcntl.h>
#include <unistd.h>
#include <cerrno>
#include <cstdio>
#include <map>
#include <memory>

int main(int argc, char **argv)
{
    // Always use native Wayland. Chromium's sandbox remains enabled.
    qputenv("QT_QPA_PLATFORM", "wayland");
    QtWebEngineQuick::initialize();
    QGuiApplication app(argc, argv);
    qInstallMessageHandler([](QtMsgType type, const QMessageLogContext &, const QString &message) {
        fprintf(stderr, "[Better Wallpaper Web] %s\n", qPrintable(message));
        fflush(stderr);
        if (type == QtFatalMsg) std::abort();
    });
    app.setQuitOnLastWindowClosed(false);
    if (app.arguments().contains("--check")) {
        QQuickView view;
        view.setSource(QUrl("qrc:/WebWallpaper.qml"));
        QElapsedTimer timeout;
        timeout.start();
        while (view.status() == QQuickView::Loading && timeout.elapsed() < 10000)
            app.processEvents();
        for (const auto &error : view.errors())
            fprintf(stderr, "%s\n", qPrintable(error.toString()));
        return view.status() == QQuickView::Ready ? 0 : 1;
    }

    QJsonObject config;
    bool initialized = false;
    QByteArray input;
    std::map<QScreen *, std::unique_ptr<QQuickView>> views;
    const auto reconcile = [&]() {
        if (!initialized) return;
        const auto screens = app.screens();
        const auto outputs = config.value("outputs").toArray();
        const auto selected = [&](QScreen *screen) {
            if (outputs.isEmpty()) return screen == app.primaryScreen();
            for (const auto &output : outputs)
                if (output.toString() == screen->name()) return true;
            return false;
        };
        for (auto it = views.begin(); it != views.end();) {
            if (!screens.contains(it->first) || !selected(it->first) || !it->second->isVisible())
                it = views.erase(it);
            else
                ++it;
        }
        for (auto *screen : screens) {
            if (!selected(screen) || views.count(screen)) continue;
            auto view = std::make_unique<QQuickView>();
            view->setScreen(screen);
            view->setResizeMode(QQuickView::SizeRootObjectToView);
            view->setInitialProperties({
                {"sourceUrl", config.value("url").toString()},
                {"propertiesJson", QString::fromUtf8(QJsonDocument(config.value("properties").toObject()).toJson(QJsonDocument::Compact))},
                {"paused", config.value("paused").toBool()},
                {"fpsLimit", config.value("fps").toInt(60)},
                {"muted", config.value("muted").toBool()}
            });
            view->setSource(QUrl("qrc:/WebWallpaper.qml"));
            if (view->status() != QQuickView::Ready) {
                for (const auto &error : view->errors())
                    fprintf(stderr, "%s\n", qPrintable(error.toString()));
                qCritical("Failed to load web wallpaper QML");
                app.exit(1);
                return;
            }
            auto *layer = LayerShellQt::Window::get(view.get());
            layer->setScope("better-wallpaper-web");
            layer->setScreen(screen);
            layer->setLayer(LayerShellQt::Window::LayerBackground);
            layer->setAnchors(LayerShellQt::Window::Anchors(LayerShellQt::Window::AnchorTop) | LayerShellQt::Window::AnchorBottom
                              | LayerShellQt::Window::AnchorLeft | LayerShellQt::Window::AnchorRight);
            layer->setExclusiveZone(-1);
            layer->setDesiredSize(QSize(0, 0));
            layer->setKeyboardInteractivity(LayerShellQt::Window::KeyboardInteractivityNone);
            layer->setCloseOnDismissed(true);
            view->resize(screen->size());
            view->show();
            qInfo() << "Web wallpaper background created for output" << screen->name();
            views.emplace(screen, std::move(view));
        }
    };
    QObject::connect(&app, &QGuiApplication::screenRemoved, &app, [&](QScreen *screen) {
        views.erase(screen);
        qInfo() << "Web wallpaper output removed" << screen->name();
    });
    QObject::connect(&app, &QGuiApplication::screenAdded, &app, [&](QScreen *) { reconcile(); });
    const int flags = fcntl(STDIN_FILENO, F_GETFL);
    if (flags < 0 || fcntl(STDIN_FILENO, F_SETFL, flags | O_NONBLOCK) < 0) return 1;
    QTimer control;
    QObject::connect(&control, &QTimer::timeout, &app, [&]() {
        char buffer[4096];
        while (true) {
            const auto count = read(STDIN_FILENO, buffer, sizeof(buffer));
            if (count == 0) { app.quit(); return; }
            if (count < 0) {
                if (errno == EINTR) continue;
                if (errno != EAGAIN && errno != EWOULDBLOCK) app.exit(1);
                break;
            }
            input.append(buffer, count);
            if (input.size() > 1024 * 1024) { app.exit(1); return; }
        }
        int newline;
        while ((newline = input.indexOf('\n')) >= 0) {
            QJsonParseError error;
            const auto document = QJsonDocument::fromJson(input.left(newline), &error);
            input.remove(0, newline + 1);
            if (error.error != QJsonParseError::NoError || !document.isObject()) {
                qCritical("Invalid web wallpaper control message");
                app.exit(1);
                return;
            }
            const auto message = document.object();
            if (!initialized) {
                const QUrl url(message.value("url").toString());
                if (url.scheme() != "http" || url.host() != "127.0.0.1") { app.exit(1); return; }
                config = message;
                initialized = true;
                reconcile();
            } else {
                config["paused"] = message.value("paused");
                for (auto &[screen, view] : views) {
                    Q_UNUSED(screen);
                    view->rootObject()->setProperty("paused", config.value("paused").toBool());
                }
            }
        }
    });
    control.start(20);
    QTimer outputs;
    QObject::connect(&outputs, &QTimer::timeout, &app, reconcile);
    outputs.start(1000);
    return app.exec();
}
