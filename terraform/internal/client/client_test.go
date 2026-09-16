package client

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestLoginAndGetConfig(t *testing.T) {
	mux := http.NewServeMux()
	mux.HandleFunc("/api/auth/login", func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			t.Fatalf("method %s", r.Method)
		}
		_ = json.NewEncoder(w).Encode(map[string]any{
			"token":      "tok-1",
			"username":   "admin",
			"expires_in": 3600,
		})
	})
	mux.HandleFunc("/api/config", func(w http.ResponseWriter, r *http.Request) {
		if got := r.Header.Get("Authorization"); got != "Bearer tok-1" {
			t.Fatalf("auth header %q", got)
		}
		_ = json.NewEncoder(w).Encode(ProxyConfig{
			Sites:    []Site{},
			Backends: []Backend{},
			TLS:      []TlsConfig{},
		})
	})
	srv := httptest.NewServer(mux)
	defer srv.Close()

	c, err := New(Config{
		Endpoint: srv.URL,
		Username: "admin",
		Password: "admin",
	})
	if err != nil {
		t.Fatal(err)
	}
	cfg, err := c.GetConfig(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if cfg == nil {
		t.Fatal("nil config")
	}
}

func TestUpsertAndRemoveSite(t *testing.T) {
	cfg := &ProxyConfig{}
	UpsertSite(cfg, Site{
		Host:    "app.example.com",
		Backend: "app",
		Routes:  []PathRewrite{{Path: "/", PathType: "Prefix"}},
	}, "http://127.0.0.1:8080")

	if len(cfg.Sites) != 1 || len(cfg.Backends) != 1 {
		t.Fatalf("sites=%d backends=%d", len(cfg.Sites), len(cfg.Backends))
	}
	if cfg.Backends[0].Upstreams[0].Addr != "http://127.0.0.1:8080" {
		t.Fatalf("upstream %q", cfg.Backends[0].Upstreams[0].Addr)
	}

	UpsertSite(cfg, Site{
		Host:    "app.example.com",
		Backend: "app",
		Routes:  []PathRewrite{{Path: "/api", PathType: "Prefix"}},
	}, "http://127.0.0.1:9090")
	if len(cfg.Sites) != 1 {
		t.Fatalf("expected single site, got %d", len(cfg.Sites))
	}
	if cfg.Sites[0].Routes[0].Path != "/api" {
		t.Fatalf("route path %q", cfg.Sites[0].Routes[0].Path)
	}
	if cfg.Backends[0].Upstreams[0].Addr != "http://127.0.0.1:9090" {
		t.Fatalf("updated upstream %q", cfg.Backends[0].Upstreams[0].Addr)
	}

	if !RemoveSite(cfg, "app.example.com") {
		t.Fatal("remove failed")
	}
	if FindSite(cfg, "app.example.com") != nil {
		t.Fatal("site still present")
	}
}
