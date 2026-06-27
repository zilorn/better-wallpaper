// SPDX-License-Identifier: MIT

import QtQuick
import QtMultimedia
import org.kde.plasma.plasmoid

WallpaperItem {
    id: root

    readonly property string mediaUrl: "http://127.0.0.1:17321/api/v1/wallpaper/media"

    Component.onCompleted: {
        console.info("[Better Wallpaper] Plasma 壁纸实例已创建，连接本机 daemon")
        player.play()
    }
    Component.onDestruction: console.info("[Better Wallpaper] Plasma 壁纸实例已销毁")

    Rectangle {
        anchors.fill: parent
        color: "black"
    }

    VideoOutput {
        id: videoOutput
        anchors.fill: parent
        fillMode: VideoOutput.PreserveAspectCrop
    }

    MediaPlayer {
        id: player
        source: root.mediaUrl
        videoOutput: videoOutput
        loops: MediaPlayer.Infinite

        audioOutput: AudioOutput {
            muted: true
        }

        onErrorOccurred: (error, errorString) => {
            console.error("[Better Wallpaper] Plasma 视频播放失败：" + errorString)
            retryTimer.restart()
        }
        onPlaybackStateChanged: console.info("[Better Wallpaper] Plasma 播放状态：" + playbackState)
    }

    Timer {
        id: retryTimer
        interval: 3000
        repeat: false
        onTriggered: {
            console.warn("[Better Wallpaper] Plasma 正在重试连接 daemon")
            player.source = ""
            player.source = root.mediaUrl + "?retry=" + Date.now()
            player.play()
        }
    }
}
