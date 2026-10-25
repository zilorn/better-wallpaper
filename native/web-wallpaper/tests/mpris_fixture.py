"""Owned MPRIS peer, run on an isolated session bus by protocol.py."""
import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib
DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
name = dbus.service.BusName('org.mpris.MediaPlayer2.betterwallpaper_a5', bus)
class Player(dbus.service.Object):
    def __init__(self):
        super().__init__(bus, '/org/mpris/MediaPlayer2')
        self.status, self.title, self.position = 'Playing', 'Owned A5 Track', 7250000
    @dbus.service.method('org.freedesktop.DBus.Properties', in_signature='s', out_signature='a{sv}')
    def GetAll(self, interface):
        assert interface == 'org.mpris.MediaPlayer2.Player'
        return {'PlaybackStatus':dbus.String(self.status), 'Position':dbus.Int64(self.position),
                'Metadata':dbus.Dictionary({'mpris:trackid':dbus.ObjectPath('/owned/track'),
                    'mpris:length':dbus.Int64(123000000), 'xesam:title':dbus.String(self.title),
                    'xesam:artist':dbus.Array(['Owned Artist'],signature='s'),
                    'xesam:album':dbus.String('Owned Album')},signature='sv')}
    @dbus.service.method('org.betterwallpaper.Test', in_signature='ssd', out_signature='')
    def Change(self, status, title, position):
        self.status, self.title, self.position = str(status), str(title), int(position*1000000)
player = Player()
print('READY',flush=True)
GLib.MainLoop().run()
