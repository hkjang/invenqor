FROM node:22-alpine AS web
WORKDIR /src/web
COPY web/package*.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM golang:1.25-alpine AS server
WORKDIR /src
# 떠 있는 컨테이너에서 어느 소스로 만든 것인지 알 수 있어야 한다. 이 셋을
# 넣지 않으면 이미지는 version.go 에 적힌 값을 그대로 들고 나가, Commit 과
# BuildTime 이 언제나 "unknown" 이다 — 장애가 났을 때 무엇이 떠 있는지 알
# 방법이 없다.
ARG VERSION=dev
ARG COMMIT=unknown
ARG BUILT_AT=unknown
COPY server/ ./server/
COPY --from=web /src/web/dist/ ./server/internal/webui/dist/
RUN cd server && CGO_ENABLED=0 go build -trimpath \
    -ldflags="-s -w \
      -X github.com/hkjang/invenqor/server/internal/version.Version=${VERSION} \
      -X github.com/hkjang/invenqor/server/internal/version.Commit=${COMMIT} \
      -X github.com/hkjang/invenqor/server/internal/version.BuildTime=${BUILT_AT}" \
    -o /out/invenqor-server ./cmd/invenqor-server \
    && mkdir -p /out/state

FROM gcr.io/distroless/static-debian12:nonroot
ARG VERSION=dev
ARG COMMIT=unknown
ARG BUILT_AT=unknown
LABEL org.opencontainers.image.title="Invenqor" \
      org.opencontainers.image.description="Invenqor IT asset management server" \
      org.opencontainers.image.source="https://github.com/hkjang/invenqor" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${COMMIT}" \
      org.opencontainers.image.created="${BUILT_AT}"
COPY --from=server /out/invenqor-server /usr/local/bin/invenqor-server
COPY --chown=65532:65532 --from=server /out/state /var/lib/invenqor-server
VOLUME ["/var/lib/invenqor-server"]
EXPOSE 7070
ENV INVENQOR_LISTEN_ADDRESS=0.0.0.0:7070
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/invenqor-server"]
