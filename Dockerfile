FROM node:22-alpine AS web
WORKDIR /src/web
COPY web/package*.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM golang:1.25-alpine AS server
WORKDIR /src
COPY server/ ./server/
COPY --from=web /src/web/dist/ ./server/internal/webui/dist/
RUN cd server && CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /out/invenqor-server ./cmd/invenqor-server \
    && mkdir -p /out/state

FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=server /out/invenqor-server /usr/local/bin/invenqor-server
COPY --chown=65532:65532 --from=server /out/state /var/lib/invenqor-server
VOLUME ["/var/lib/invenqor-server"]
EXPOSE 7070
ENV INVENQOR_LISTEN_ADDRESS=0.0.0.0:7070
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/invenqor-server"]
