import os  # noqa


def size(path):
    return os.stat(path).st_size
