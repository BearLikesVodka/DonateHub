"""Desktop shell: a Qt window around the local settings page, a tray icon, and a single running instance."""

import logging
import os
import signal
import sys
from pathlib import Path

from PySide6.QtCore import QObject, QTimer, QUrl, Signal
from PySide6.QtGui import QAction, QCloseEvent, QDesktopServices, QGuiApplication, QIcon
from PySide6.QtNetwork import QLocalServer, QLocalSocket
from PySide6.QtWebEngineCore import QWebEnginePage, QWebEngineSettings
from PySide6.QtWebEngineWidgets import QWebEngineView
from PySide6.QtWidgets import QApplication, QInputDialog, QMainWindow, QMenu, QMessageBox, QSystemTrayIcon

from .app import create_app
from .config import port_free
from .server import ServerThread
from .store import Store

log = logging.getLogger(__name__)

APP_NAME = "DonateHub"
ICON_PATH = Path(__file__).parent / "static" / "icon.svg"
LOCAL_HOSTS = {"127.0.0.1", "localhost"}
# Pages that must open in the system browser even though they are served locally.
EXTERNAL_LOCAL_PATHS = {"/auth/da/start"}


def instance_name() -> str:
    return f"donatehub-{os.getuid()}"


def open_external(url: QUrl) -> None:
    QDesktopServices.openUrl(url)


def is_internal(url: QUrl) -> bool:
    if url.scheme() in ("about", "data", "blob"):
        return True
    return url.host() in LOCAL_HOSTS and url.path() not in EXTERNAL_LOCAL_PATHS


def notify_running_instance(name: str) -> bool:
    sock = QLocalSocket()
    sock.connectToServer(name)
    if not sock.waitForConnected(300):
        return False
    sock.write(b"show")
    sock.waitForBytesWritten(300)
    sock.disconnectFromServer()
    return True


class SingleInstance(QObject):
    show_requested = Signal()

    def __init__(self, name: str):
        super().__init__()
        self.name = name
        self.server = QLocalServer(self)
        self.server.newConnection.connect(self._on_connection)

    def listen(self) -> None:
        QLocalServer.removeServer(self.name)  # stale socket left by a crash
        if not self.server.listen(self.name):
            log.warning("single-instance socket: %s", self.server.errorString())

    def _on_connection(self) -> None:
        while (conn := self.server.nextPendingConnection()) is not None:
            conn.readyRead.connect(lambda c=conn: self._read(c))
            conn.disconnected.connect(conn.deleteLater)
            if conn.bytesAvailable():
                self._read(conn)

    def _read(self, conn: QLocalSocket) -> None:
        if b"show" in bytes(conn.readAll().data()):
            self.show_requested.emit()


class _ExternalLinkPage(QWebEnginePage):
    """Receives target=_blank navigations and hands them to the system browser."""

    def acceptNavigationRequest(self, url, nav_type, is_main_frame):
        open_external(url)
        self.deleteLater()
        return False


class AppPage(QWebEnginePage):
    def __init__(self, parent: QObject | None = None):
        super().__init__(parent)
        s = self.settings()
        s.setAttribute(QWebEngineSettings.WebAttribute.JavascriptCanAccessClipboard, True)
        s.setAttribute(QWebEngineSettings.WebAttribute.JavascriptCanPaste, True)
        if hasattr(self, "permissionRequested"):  # Qt >= 6.8
            self.permissionRequested.connect(self._on_permission)

    def _on_permission(self, permission) -> None:
        if permission.origin().host() in LOCAL_HOSTS:
            permission.grant()
        else:
            permission.deny()

    def acceptNavigationRequest(self, url, nav_type, is_main_frame):
        if is_main_frame and not is_internal(url):
            open_external(url)
            return False
        return True

    def createWindow(self, window_type):
        return _ExternalLinkPage(self)


def request_quit(window: "MainWindow") -> None:
    """Really quit. QApplication.quit() in Qt 6 first asks windows to close, and ours refuses (it hides to tray)."""
    window.allow_close = True
    window.close()
    QApplication.exit(0)


class MainWindow(QMainWindow):
    hidden_to_tray = Signal()

    def __init__(self, url: str, tray_available: bool = True):
        super().__init__()
        self.allow_close = False
        self.tray_available = tray_available
        self.setWindowTitle(APP_NAME)
        self.setWindowIcon(QIcon(str(ICON_PATH)))
        self.resize(1280, 860)
        self.view = QWebEngineView(self)
        self.view.setPage(AppPage(self.view))
        self.view.setUrl(QUrl(url))
        self.setCentralWidget(self.view)

    def closeEvent(self, event: QCloseEvent) -> None:
        if self.allow_close:
            event.accept()
            return
        # Closing must never stop the server: OBS keeps reading the overlay.
        event.ignore()
        if self.tray_available:
            self.hide()
            self.hidden_to_tray.emit()
        else:
            self.showMinimized()

    def bring_to_front(self) -> None:
        if self.isMinimized():
            self.showNormal()
        self.show()
        self.raise_()
        self.activateWindow()


class Tray(QObject):
    def __init__(self, window: MainWindow, port: int, icon: QIcon):
        super().__init__()
        self.window = window
        self.overlay_url = f"http://127.0.0.1:{port}/overlay"
        self.told_about_tray = False
        self.icon = QSystemTrayIcon(icon, self)
        self.icon.setToolTip(APP_NAME)
        menu = QMenu()
        for text, slot in (("Открыть настройки", window.bring_to_front),
                           ("Скопировать ссылку для OBS", self.copy_overlay_url)):
            action = QAction(text, menu)
            action.triggered.connect(slot)
            menu.addAction(action)
        menu.addSeparator()
        quit_action = QAction("Выход", menu)
        quit_action.triggered.connect(lambda: request_quit(window))
        menu.addAction(quit_action)
        self.menu = menu
        self.icon.setContextMenu(menu)
        self.icon.activated.connect(self._on_activated)
        window.hidden_to_tray.connect(self._on_hidden)
        self.icon.show()

    def _on_activated(self, reason) -> None:
        if reason == QSystemTrayIcon.ActivationReason.Trigger:
            if self.window.isVisible() and not self.window.isMinimized():
                self.window.hide()
            else:
                self.window.bring_to_front()

    def _on_hidden(self) -> None:
        if not self.told_about_tray:
            self.told_about_tray = True
            self.icon.showMessage(APP_NAME, "Приложение работает в трее. Оверлей в OBS продолжает обновляться.",
                                  self.icon.icon(), 5000)

    def copy_overlay_url(self) -> None:
        QGuiApplication.clipboard().setText(self.overlay_url)
        self.icon.showMessage(APP_NAME, f"Ссылка скопирована: {self.overlay_url}", self.icon.icon(), 3000)


def ask_new_port(store: Store, port: int) -> None:
    new_port, ok = QInputDialog.getInt(
        None, APP_NAME,
        f"Порт {port} занят другой программой.\nУкажите другой порт (1024–65535):",
        port + 1 if port < 65535 else 47321, 1024, 65535,
    )
    if ok:
        store.set_setting("port", new_port)
        QMessageBox.information(
            None, APP_NAME,
            f"Порт {new_port} сохранён. Запустите приложение снова.\n"
            "Не забудьте обновить ссылку в OBS и Redirect URL в приложении DonationAlerts.",
        )


def run(store: Store, port: int, hidden: bool = False) -> int:
    app = QApplication(sys.argv)
    app.setApplicationName(APP_NAME)
    app.setDesktopFileName("donatehub")
    app.setQuitOnLastWindowClosed(False)
    icon = QIcon(str(ICON_PATH))
    app.setWindowIcon(icon)

    name = instance_name()
    if notify_running_instance(name):
        return 0

    if not port_free(port):
        ask_new_port(store, port)
        return 1

    server = ServerThread(create_app(store, port=port), port)
    server.start()
    if not server.wait_ready(15):
        QMessageBox.critical(None, APP_NAME, f"Не удалось запустить сервер на порту {port}.")
        server.stop()
        return 1

    instance = SingleInstance(name)
    instance.listen()

    tray_available = QSystemTrayIcon.isSystemTrayAvailable()
    window = MainWindow(f"http://127.0.0.1:{port}/", tray_available=tray_available)
    instance.show_requested.connect(window.bring_to_front)
    tray = Tray(window, port, icon) if tray_available else None
    if not hidden or tray is None:
        window.show()

    app.aboutToQuit.connect(server.stop)
    # Quit cleanly on logout (SIGTERM) or Ctrl+C; the timer lets Python run signal handlers inside Qt's loop.
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, lambda *_: request_quit(window))
    tick = QTimer()
    tick.start(250)
    tick.timeout.connect(lambda: None)
    return app.exec()
