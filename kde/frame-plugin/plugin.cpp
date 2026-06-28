#include "frameitem.h"
#include <QQmlExtensionPlugin>
#include <qqml.h>

class BetterWallpaperPlugin final : public QQmlExtensionPlugin {
    Q_OBJECT
    Q_PLUGIN_METADATA(IID QQmlExtensionInterface_iid)
public:
    void registerTypes(const char *uri) override {
        qmlRegisterType<FrameItem>(uri, 1, 0, "SharedFrameItem");
    }
};

#include "plugin.moc"
