"""A stand-in for a desktop's system tray, for tools/e2e and tools/shot.

It owns org.kde.StatusNotifierWatcher on the session bus, as KDE Plasma's
does, says a host shows the items, and prints "registered NAME" for each item
that registers, NAME being the item's bus name. Clicks and menu choices are
then made with gdbus, as a tray would make them.
"""

import sys

from gi.repository import Gio, GLib

INTERFACE = """
<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem">
      <arg name="service" type="s" direction="in"/>
    </method>
    <method name="RegisterStatusNotifierHost">
      <arg name="service" type="s" direction="in"/>
    </method>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
    <signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
  </interface>
</node>
"""

items = []


def method_call(connection, sender, path, interface, method, parameters, invocation):
    if method == "RegisterStatusNotifierItem":
        (service,) = parameters.unpack()
        # An item may register by its object path, on its own connection.
        name = sender if service.startswith("/") else service
        items.append(name)
        print("registered", name, flush=True)
        connection.emit_signal(
            None, path, interface, "StatusNotifierItemRegistered", GLib.Variant("(s)", (name,))
        )
    invocation.return_value(None)


def get_property(connection, sender, path, interface, name):
    return {
        "RegisteredStatusNotifierItems": GLib.Variant("as", items),
        "IsStatusNotifierHostRegistered": GLib.Variant("b", True),
        "ProtocolVersion": GLib.Variant("i", 0),
    }[name]


def bus_acquired(connection, name):
    info = Gio.DBusNodeInfo.new_for_xml(INTERFACE).interfaces[0]
    connection.register_object("/StatusNotifierWatcher", info, method_call, get_property, None)


Gio.bus_own_name(
    Gio.BusType.SESSION,
    "org.kde.StatusNotifierWatcher",
    Gio.BusNameOwnerFlags.NONE,
    bus_acquired,
    lambda connection, name: print("ready", flush=True),
    lambda connection, name: sys.exit("org.kde.StatusNotifierWatcher is taken"),
)
GLib.MainLoop().run()
