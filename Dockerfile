FROM python:3.12-slim-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg iputils-ping iputils-arping \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY server ./server
RUN python server/stamp.py server/camd/web \
    && pip install --no-cache-dir ./server \
    && useradd --system --home /data --shell /usr/sbin/nologin camd \
    && mkdir /data && chown camd:camd /data

ENV CAMD_LISTEN_HOST=0.0.0.0 \
    CAMD_DATA_DIR=/data \
    PYTHONUNBUFFERED=1

USER camd
VOLUME /data
EXPOSE 8180
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s \
    CMD python -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8180/healthz', timeout=3)"
CMD ["python", "-m", "camd"]
