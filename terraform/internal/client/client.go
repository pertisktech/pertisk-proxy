package client

import (
	"bytes"
	"context"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

type Client struct {
	endpoint   string
	token      string
	httpClient *http.Client
}

type Config struct {
	Endpoint string
	Token    string
	Username string
	Password string
	Insecure bool
}

func New(cfg Config) (*Client, error) {
	endpoint := strings.TrimRight(strings.TrimSpace(cfg.Endpoint), "/")
	if endpoint == "" {
		return nil, fmt.Errorf("endpoint is required")
	}
	if _, err := url.ParseRequestURI(endpoint); err != nil {
		return nil, fmt.Errorf("invalid endpoint: %w", err)
	}

	transport := http.DefaultTransport.(*http.Transport).Clone()
	if cfg.Insecure {
		transport.TLSClientConfig = &tls.Config{InsecureSkipVerify: true} //nolint:gosec
	}

	c := &Client{
		endpoint: endpoint,
		token:    strings.TrimSpace(cfg.Token),
		httpClient: &http.Client{
			Timeout:   30 * time.Second,
			Transport: transport,
		},
	}

	if c.token == "" {
		if strings.TrimSpace(cfg.Username) == "" || cfg.Password == "" {
			return nil, fmt.Errorf("username and password are required when token is not set")
		}
		if err := c.Login(context.Background(), cfg.Username, cfg.Password); err != nil {
			return nil, err
		}
	}

	return c, nil
}

type loginRequest struct {
	Username string `json:"username"`
	Password string `json:"password"`
}

type loginResponse struct {
	Token     string `json:"token"`
	Username  string `json:"username"`
	ExpiresIn int64  `json:"expires_in"`
}

type apiError struct {
	Error string `json:"error"`
}

func (c *Client) Login(ctx context.Context, username, password string) error {
	var out loginResponse
	if err := c.do(ctx, http.MethodPost, "/api/auth/login", loginRequest{
		Username: username,
		Password: password,
	}, &out, false); err != nil {
		return fmt.Errorf("login failed: %w", err)
	}
	c.token = out.Token
	return nil
}

func (c *Client) do(ctx context.Context, method, path string, body any, out any, auth bool) error {
	var reader io.Reader
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return err
		}
		reader = bytes.NewReader(b)
	}

	req, err := http.NewRequestWithContext(ctx, method, c.endpoint+path, reader)
	if err != nil {
		return err
	}
	req.Header.Set("Accept", "application/json")
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if auth && c.token != "" {
		req.Header.Set("Authorization", "Bearer "+c.token)
	}

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		var ae apiError
		_ = json.Unmarshal(data, &ae)
		if ae.Error != "" {
			return fmt.Errorf("%s %s: %s (%s)", method, path, ae.Error, resp.Status)
		}
		if len(data) > 0 {
			return fmt.Errorf("%s %s: %s: %s", method, path, resp.Status, strings.TrimSpace(string(data)))
		}
		return fmt.Errorf("%s %s: %s", method, path, resp.Status)
	}

	if out == nil || resp.StatusCode == http.StatusNoContent || len(data) == 0 {
		return nil
	}
	if err := json.Unmarshal(data, out); err != nil {
		return fmt.Errorf("decode %s %s: %w", method, path, err)
	}
	return nil
}

func (c *Client) get(ctx context.Context, path string, out any) error {
	return c.do(ctx, http.MethodGet, path, nil, out, true)
}

func (c *Client) post(ctx context.Context, path string, body, out any) error {
	return c.do(ctx, http.MethodPost, path, body, out, true)
}

func (c *Client) put(ctx context.Context, path string, body, out any) error {
	return c.do(ctx, http.MethodPut, path, body, out, true)
}

func (c *Client) delete(ctx context.Context, path string, out any) error {
	return c.do(ctx, http.MethodDelete, path, nil, out, true)
}

// --- Config / sites ---

type Upstream struct {
	Addr   string `json:"addr"`
	Weight *int   `json:"weight,omitempty"`
}

type Backend struct {
	Name               string     `json:"name"`
	Upstreams          []Upstream `json:"upstreams"`
	Algorithm          string     `json:"algorithm,omitempty"`
	HealthPath         *string    `json:"health_path,omitempty"`
	HealthIntervalSecs *int64     `json:"health_interval_secs,omitempty"`
}

type PathRewrite struct {
	Path     string  `json:"path"`
	PathType string  `json:"path_type,omitempty"`
	Rewrite  *string `json:"rewrite,omitempty"`
	Upstream *string `json:"upstream,omitempty"`
}

type GeoIP struct {
	Enabled        bool     `json:"enabled,omitempty"`
	AllowCountries []string `json:"allow_countries,omitempty"`
	DenyCountries  []string `json:"deny_countries,omitempty"`
	AllowASNs      []int64  `json:"allow_asns,omitempty"`
	DenyASNs       []int64  `json:"deny_asns,omitempty"`
}

type WafRule struct {
	ID            string   `json:"id"`
	Enabled       *bool    `json:"enabled,omitempty"`
	Action        string   `json:"action,omitempty"`
	Methods       []string `json:"methods,omitempty"`
	PathContains  string   `json:"path_contains,omitempty"`
	QueryContains string   `json:"query_contains,omitempty"`
	UAContains    string   `json:"ua_contains,omitempty"`
}

type WafConfig struct {
	Enabled         *bool     `json:"enabled,omitempty"`
	UseBuiltinRules *bool     `json:"use_builtin_rules,omitempty"`
	Rules           []WafRule `json:"rules,omitempty"`
}

type BotConfig struct {
	Enabled         *bool  `json:"enabled,omitempty"`
	ChallengeScore  *int64 `json:"challenge_score,omitempty"`
	BlockScore      *int64 `json:"block_score,omitempty"`
	RateLimitPerMin *int64 `json:"rate_limit_per_min,omitempty"`
}

type CaptchaConfig struct {
	Enabled       *bool  `json:"enabled,omitempty"`
	CookieTTLSecs *int64 `json:"cookie_ttl_secs,omitempty"`
}

type Security struct {
	WAF     *WafConfig     `json:"waf,omitempty"`
	Bot     *BotConfig     `json:"bot,omitempty"`
	Captcha *CaptchaConfig `json:"captcha,omitempty"`
}

type Site struct {
	Host             string        `json:"host"`
	Backend          string        `json:"backend"`
	Routes           []PathRewrite `json:"routes"`
	ForwardClientIP  *bool         `json:"forward_client_ip,omitempty"`
	AccessListID     *string       `json:"access_list_id,omitempty"`
	WafPolicyID      *string       `json:"waf_policy_id,omitempty"`
	GeoIP            *GeoIP        `json:"geoip,omitempty"`
	Security         *Security     `json:"security,omitempty"`
	IngressNamespace *string       `json:"ingress_namespace,omitempty"`
	IngressName      *string       `json:"ingress_name,omitempty"`
	K8sResourceKind  *string       `json:"k8s_resource_kind,omitempty"`
}

type TlsSource struct {
	Type            string            `json:"type"`
	Cert            string            `json:"cert,omitempty"`
	Key             string            `json:"key,omitempty"`
	Email           *string           `json:"email,omitempty"`
	Challenge       string            `json:"challenge,omitempty"`
	DNSProvider     *string           `json:"dns_provider,omitempty"`
	DNSProviderType *string           `json:"dns_provider_type,omitempty"`
	DNSCredentials  map[string]string `json:"dns_credentials,omitempty"`
}

type TlsConfig struct {
	Hosts     []string  `json:"hosts"`
	Source    TlsSource `json:"source"`
	ExpiresAt *string   `json:"expires_at,omitempty"`
}

type AccessList struct {
	ID          string  `json:"id"`
	Name        string  `json:"name"`
	Description *string `json:"description,omitempty"`
	GeoIP       GeoIP   `json:"geoip"`
}

type WafPolicy struct {
	ID          string   `json:"id"`
	Name        string   `json:"name"`
	Description *string  `json:"description,omitempty"`
	Security    Security `json:"security"`
}

type ProxyConfig struct {
	Sites       []Site       `json:"sites"`
	Backends    []Backend    `json:"backends"`
	TLS         []TlsConfig  `json:"tls"`
	AccessLists []AccessList `json:"access_lists,omitempty"`
	WafPolicies []WafPolicy  `json:"waf_policies,omitempty"`
	ProxyLog    *bool        `json:"proxy_log,omitempty"`
	AcmeEmail   *string      `json:"acme_email,omitempty"`
}

type PutConfigResponse struct {
	OK         bool `json:"ok"`
	RouteCount int  `json:"route_count"`
}

func (c *Client) GetConfig(ctx context.Context) (*ProxyConfig, error) {
	var cfg ProxyConfig
	if err := c.get(ctx, "/api/config", &cfg); err != nil {
		return nil, err
	}
	return &cfg, nil
}

func (c *Client) PutConfig(ctx context.Context, cfg *ProxyConfig) (*PutConfigResponse, error) {
	var out PutConfigResponse
	if err := c.put(ctx, "/api/config", cfg, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// --- DNS providers ---

type DnsProvider struct {
	ID           string            `json:"id"`
	Name         string            `json:"name"`
	ProviderType string            `json:"provider_type"`
	Credentials  map[string]string `json:"credentials,omitempty"`
	CreatedAt    string            `json:"created_at,omitempty"`
}

type DnsProviderInput struct {
	Name         string            `json:"name"`
	ProviderType string            `json:"provider_type"`
	Credentials  map[string]string `json:"credentials,omitempty"`
}

type IDResponse struct {
	ID string `json:"id"`
}

type OKResponse struct {
	OK bool `json:"ok"`
}

func (c *Client) ListDnsProviders(ctx context.Context) ([]DnsProvider, error) {
	var out []DnsProvider
	if err := c.get(ctx, "/api/dns-providers", &out); err != nil {
		return nil, err
	}
	return out, nil
}

func (c *Client) GetDnsProvider(ctx context.Context, id string) (*DnsProvider, error) {
	var out DnsProvider
	if err := c.get(ctx, "/api/dns-providers/"+url.PathEscape(id), &out); err != nil {
		return nil, err
	}
	return &out, nil
}

func (c *Client) CreateDnsProvider(ctx context.Context, in DnsProviderInput) (string, error) {
	var out IDResponse
	if err := c.post(ctx, "/api/dns-providers", in, &out); err != nil {
		return "", err
	}
	return out.ID, nil
}

func (c *Client) UpdateDnsProvider(ctx context.Context, id string, in DnsProviderInput) error {
	var out OKResponse
	return c.put(ctx, "/api/dns-providers/"+url.PathEscape(id), in, &out)
}

func (c *Client) DeleteDnsProvider(ctx context.Context, id string) error {
	var out OKResponse
	return c.delete(ctx, "/api/dns-providers/"+url.PathEscape(id), &out)
}

// --- Access lists ---

type AccessListInput struct {
	Name        string  `json:"name"`
	Description *string `json:"description,omitempty"`
	GeoIP       *GeoIP  `json:"geoip,omitempty"`
}

func (c *Client) ListAccessLists(ctx context.Context) ([]AccessList, error) {
	var out []AccessList
	if err := c.get(ctx, "/api/access-lists", &out); err != nil {
		return nil, err
	}
	return out, nil
}

func (c *Client) GetAccessList(ctx context.Context, id string) (*AccessList, error) {
	var out AccessList
	if err := c.get(ctx, "/api/access-lists/"+url.PathEscape(id), &out); err != nil {
		return nil, err
	}
	return &out, nil
}

func (c *Client) CreateAccessList(ctx context.Context, in AccessListInput) (string, error) {
	var out IDResponse
	if err := c.post(ctx, "/api/access-lists", in, &out); err != nil {
		return "", err
	}
	return out.ID, nil
}

func (c *Client) UpdateAccessList(ctx context.Context, id string, in AccessListInput) error {
	var out OKResponse
	return c.put(ctx, "/api/access-lists/"+url.PathEscape(id), in, &out)
}

func (c *Client) DeleteAccessList(ctx context.Context, id string) error {
	var out OKResponse
	return c.delete(ctx, "/api/access-lists/"+url.PathEscape(id), &out)
}

// --- WAF policies ---

type WafPolicyInput struct {
	Name        string    `json:"name"`
	Description *string   `json:"description,omitempty"`
	Security    *Security `json:"security,omitempty"`
}

func (c *Client) ListWafPolicies(ctx context.Context) ([]WafPolicy, error) {
	var out []WafPolicy
	if err := c.get(ctx, "/api/waf-policies", &out); err != nil {
		return nil, err
	}
	return out, nil
}

func (c *Client) GetWafPolicy(ctx context.Context, id string) (*WafPolicy, error) {
	var out WafPolicy
	if err := c.get(ctx, "/api/waf-policies/"+url.PathEscape(id), &out); err != nil {
		return nil, err
	}
	return &out, nil
}

func (c *Client) CreateWafPolicy(ctx context.Context, in WafPolicyInput) (string, error) {
	var out IDResponse
	if err := c.post(ctx, "/api/waf-policies", in, &out); err != nil {
		return "", err
	}
	return out.ID, nil
}

func (c *Client) UpdateWafPolicy(ctx context.Context, id string, in WafPolicyInput) error {
	var out OKResponse
	return c.put(ctx, "/api/waf-policies/"+url.PathEscape(id), in, &out)
}

func (c *Client) DeleteWafPolicy(ctx context.Context, id string) error {
	var out OKResponse
	return c.delete(ctx, "/api/waf-policies/"+url.PathEscape(id), &out)
}

// UpsertSite merges a site into config and optionally ensures a backend exists.
func UpsertSite(cfg *ProxyConfig, site Site, backendUpstream string) {
	if backendUpstream != "" {
		ensureBackend(cfg, site.Backend, backendUpstream)
	}
	for i := range cfg.Sites {
		if cfg.Sites[i].Host == site.Host {
			cfg.Sites[i] = site
			return
		}
	}
	cfg.Sites = append(cfg.Sites, site)
}

func RemoveSite(cfg *ProxyConfig, host string) bool {
	for i := range cfg.Sites {
		if cfg.Sites[i].Host == host {
			cfg.Sites = append(cfg.Sites[:i], cfg.Sites[i+1:]...)
			return true
		}
	}
	return false
}

func FindSite(cfg *ProxyConfig, host string) *Site {
	for i := range cfg.Sites {
		if cfg.Sites[i].Host == host {
			return &cfg.Sites[i]
		}
	}
	return nil
}

func ensureBackend(cfg *ProxyConfig, name, addr string) {
	for i := range cfg.Backends {
		if cfg.Backends[i].Name == name {
			cfg.Backends[i].Upstreams = []Upstream{{Addr: addr}}
			return
		}
	}
	cfg.Backends = append(cfg.Backends, Backend{
		Name:      name,
		Upstreams: []Upstream{{Addr: addr}},
	})
}
