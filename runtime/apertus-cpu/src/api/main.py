import threading
from ..config import Config
from ..application.completion import CompletionService
from ..infrastructure.model import ApertusModel
from ..infrastructure.safe_logging import configure_logging
from .server import PrivateServer


def main():
    config = Config.from_env()
    configure_logging(config)
    backend = ApertusModel(config)
    server = PrivateServer((config.host, config.port), config, CompletionService(config, backend))
    threading.Thread(target=backend.load, name="model-loader", daemon=True).start()
    try:
        server.serve_forever()
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
