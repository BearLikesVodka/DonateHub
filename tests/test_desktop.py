import os
import uuid

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
os.environ.setdefault("QTWEBENGINE_CHROMIUM_FLAGS", "--no-sandbox")

import pytest
from PySide6.QtCore import QUrl
from PySide6.QtWebEngineCore import QWebEnginePage
from PySide6.QtWidgets import QApplication

from donation_goal import desktop


@pytest.fixture(scope="module")
def qapp():
    app = QApplication.instance() or QApplication([])
    yield app


def pump(app, until, timeout_ms=3000):
    from PySide6.QtCore import QElapsedTimer
    t = QElapsedTimer()
    t.start()
    while not until() and t.elapsed() < timeout_ms:
        app.processEvents()
    return until()


def test_main_window_close_hides(qapp):
    w = desktop.MainWindow("about:blank")
    hidden = []
    w.hidden_to_tray.connect(lambda: hidden.append(1))
    w.show()
    assert w.isVisible()
    w.close()
    assert not w.isVisible()
    assert hidden == [1]


def test_single_instance_forwards_show(qapp):
    name = f"donation-goal-test-{uuid.uuid4().hex}"
    assert desktop.notify_running_instance(name) is False
    inst = desktop.SingleInstance(name)
    inst.listen()
    shown = []
    inst.show_requested.connect(lambda: shown.append(1))
    assert desktop.notify_running_instance(name) is True
    assert pump(qapp, lambda: shown == [1])
    # In the app the instance lives until exit; here, let pending socket events drain before it is destroyed.
    pump(qapp, lambda: False, 200)
    inst.server.close()
    pump(qapp, lambda: False, 100)


def test_external_navigation_opens_browser(qapp, monkeypatch):
    opened = []
    monkeypatch.setattr(desktop, "open_external", lambda url: opened.append(url.toString()))
    page = desktop.AppPage()
    nav = QWebEnginePage.NavigationType.NavigationTypeLinkClicked
    assert page.acceptNavigationRequest(QUrl("https://www.donationalerts.com/application/clients"), nav, True) is False
    assert page.acceptNavigationRequest(QUrl("http://127.0.0.1:47321/auth/da/start"), nav, True) is False
    assert page.acceptNavigationRequest(QUrl("http://127.0.0.1:47321/#design"), nav, True) is True
    assert opened == ["https://www.donationalerts.com/application/clients", "http://127.0.0.1:47321/auth/da/start"]


def test_request_quit_exits_event_loop_despite_tray_window(qapp):
    from PySide6.QtCore import QTimer

    w = desktop.MainWindow("about:blank")
    w.show()
    QTimer.singleShot(50, lambda: desktop.request_quit(w))
    guard = QTimer()
    guard.setSingleShot(True)
    guard.timeout.connect(lambda: qapp.exit(99))  # if quitting is swallowed, bail out with 99
    guard.start(3000)
    assert qapp.exec() == 0
    guard.stop()
    assert not w.isVisible()
