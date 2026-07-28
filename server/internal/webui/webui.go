package webui

import (
	"embed"
	"io/fs"
	"net/http"
	"strings"
)

// console contains the production React build.
//
//go:embed dist
var console embed.FS

func Handler() http.Handler {
	root, err := fs.Sub(console, "dist")
	if err != nil {
		panic(err)
	}
	files := http.FileServer(http.FS(root))
	return http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		if strings.HasPrefix(request.URL.Path, "/api/") ||
			strings.HasPrefix(request.URL.Path, "/v1/") ||
			strings.HasPrefix(request.URL.Path, "/health/") {
			response.Header().Set("Content-Type", "application/json")
			response.WriteHeader(http.StatusNotFound)
			_, _ = response.Write([]byte(`{"error":{"code":"NOT_FOUND","message":"The requested API resource does not exist."}}`))
			return
		}
		path := strings.TrimPrefix(request.URL.Path, "/")
		if path != "" {
			if _, err := fs.Stat(root, path); err == nil {
				files.ServeHTTP(response, request)
				return
			}
		}
		request.URL.Path = "/"
		response.Header().Set("Cache-Control", "no-cache")
		files.ServeHTTP(response, request)
	})
}
