import argparse
import logging

import uvicorn

from .app import create_app
from .config import data_dir, resolve_port
from .store import Store


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="donatehub", description="DonateHub: цель донатов для OBS")
    parser.add_argument("--headless", action="store_true", help="только сервер, без окна")
    parser.add_argument("--hidden", action="store_true", help="запуститься свёрнутым в трей")
    args = parser.parse_args(argv)

    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s")
    # httpx logs full request URLs at INFO, and the DonateX hub URL carries the token.
    logging.getLogger("httpx").setLevel(logging.WARNING)

    store = Store(data_dir() / "app.db")
    port = resolve_port(store)

    if args.headless:
        print(f"Настройки:  http://127.0.0.1:{port}/\nОверлей:    http://127.0.0.1:{port}/overlay", flush=True)
        uvicorn.run(create_app(store, port=port), host="127.0.0.1", port=port, log_level="warning")
        return 0

    from .desktop import run  # Qt is only needed for the desktop mode

    return run(store, port, hidden=args.hidden)


if __name__ == "__main__":
    raise SystemExit(main())
