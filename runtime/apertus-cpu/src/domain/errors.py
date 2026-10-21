class ServiceError(Exception):
    """An explicit safe error; messages must never contain request or credential data."""

    def __init__(self, status, code, message):
        super().__init__(message)
        self.status, self.code, self.message = status, code, message


def invalid(message="The request does not match the supported text completion contract."):
    return ServiceError(400, "invalid_request", message)
