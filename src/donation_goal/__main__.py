import logging
from pathlib import Path

import uvicorn

from .app import create_app
from .store import Store

DATA_DIR = Path("data")


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s")
    DATA_DIR.mkdir(exist_ok=True)
    store = Store(DATA_DIR / "app.db")
    print("Настройки:  http://127.0.0.1:8000/\nОверлей:    http://127.0.0.1:8000/overlay")
    uvicorn.run(create_app(store), host="127.0.0.1", port=8000, log_level="warning")


if __name__ == "__main__":
    main()
