package provider

import (
	"context"
	"os"
	"strings"

	"github.com/hashicorp/terraform-plugin-framework/datasource"
	"github.com/hashicorp/terraform-plugin-framework/path"
	"github.com/hashicorp/terraform-plugin-framework/provider"
	"github.com/hashicorp/terraform-plugin-framework/provider/schema"
	"github.com/hashicorp/terraform-plugin-framework/resource"
	"github.com/hashicorp/terraform-plugin-framework/types"

	"github.com/pertisktech/pertisk-proxy/terraform/internal/client"
)

var _ provider.Provider = &pertiskProvider{}

type pertiskProvider struct {
	version string
}

type providerModel struct {
	Endpoint types.String `tfsdk:"endpoint"`
	Username types.String `tfsdk:"username"`
	Password types.String `tfsdk:"password"`
	Token    types.String `tfsdk:"token"`
	Insecure types.Bool   `tfsdk:"insecure"`
}

func New(version string) func() provider.Provider {
	return func() provider.Provider {
		return &pertiskProvider{version: version}
	}
}

func (p *pertiskProvider) Metadata(_ context.Context, _ provider.MetadataRequest, resp *provider.MetadataResponse) {
	resp.TypeName = "pertisk_proxy"
	resp.Version = p.version
}

func (p *pertiskProvider) Schema(_ context.Context, _ provider.SchemaRequest, resp *provider.SchemaResponse) {
	resp.Schema = schema.Schema{
		MarkdownDescription: "Manage pertisk-proxy sites, DNS providers, access lists, and WAF policies via the management API.",
		Attributes: map[string]schema.Attribute{
			"endpoint": schema.StringAttribute{
				Optional:            true,
				MarkdownDescription: "Management API base URL, e.g. `http://127.0.0.1:9080`. Defaults to `PERTISK_ENDPOINT` or `http://127.0.0.1:9080`.",
			},
			"username": schema.StringAttribute{
				Optional:            true,
				MarkdownDescription: "Admin username. Defaults to `PERTISK_USERNAME`. Ignored when `token` is set.",
			},
			"password": schema.StringAttribute{
				Optional:            true,
				Sensitive:           true,
				MarkdownDescription: "Admin password. Defaults to `PERTISK_PASSWORD`.",
			},
			"token": schema.StringAttribute{
				Optional:            true,
				Sensitive:           true,
				MarkdownDescription: "Bearer session token from `POST /api/auth/login`. Defaults to `PERTISK_TOKEN`.",
			},
			"insecure": schema.BoolAttribute{
				Optional:            true,
				MarkdownDescription: "Skip TLS certificate verification. Defaults to `PERTISK_TLS_INSECURE`.",
			},
		},
	}
}

func (p *pertiskProvider) Configure(ctx context.Context, req provider.ConfigureRequest, resp *provider.ConfigureResponse) {
	var cfg providerModel
	resp.Diagnostics.Append(req.Config.Get(ctx, &cfg)...)
	if resp.Diagnostics.HasError() {
		return
	}

	endpoint := firstNonEmpty(cfg.Endpoint.ValueString(), os.Getenv("PERTISK_ENDPOINT"), "http://127.0.0.1:9080")
	username := firstNonEmpty(cfg.Username.ValueString(), os.Getenv("PERTISK_USERNAME"))
	password := firstNonEmpty(cfg.Password.ValueString(), os.Getenv("PERTISK_PASSWORD"))
	token := firstNonEmpty(cfg.Token.ValueString(), os.Getenv("PERTISK_TOKEN"))
	insecure := boolFromEnv(os.Getenv("PERTISK_TLS_INSECURE"))
	if !cfg.Insecure.IsNull() {
		insecure = cfg.Insecure.ValueBool()
	}

	if token == "" && (username == "" || password == "") {
		resp.Diagnostics.AddAttributeError(
			path.Root("username"),
			"Missing credentials",
			"Set token, or username and password (provider block or PERTISK_* environment variables).",
		)
		return
	}

	api, err := client.New(client.Config{
		Endpoint: endpoint,
		Username: username,
		Password: password,
		Token:    token,
		Insecure: insecure,
	})
	if err != nil {
		resp.Diagnostics.AddError("Unable to connect to pertisk-proxy", err.Error())
		return
	}
	resp.DataSourceData = api
	resp.ResourceData = api
}

func (p *pertiskProvider) Resources(_ context.Context) []func() resource.Resource {
	return []func() resource.Resource{
		NewSiteResource,
		NewDnsProviderResource,
		NewAccessListResource,
		NewWafPolicyResource,
	}
}

func (p *pertiskProvider) DataSources(_ context.Context) []func() datasource.DataSource {
	return nil
}

func configureClient(providerData any, diags interface{ AddError(string, string) }) *client.Client {
	if providerData == nil {
		return nil
	}
	api, ok := providerData.(*client.Client)
	if !ok {
		diags.AddError("Unexpected provider data", "The pertisk provider received the wrong client type.")
		return nil
	}
	return api
}

func firstNonEmpty(values ...string) string {
	for _, v := range values {
		if strings.TrimSpace(v) != "" {
			return strings.TrimSpace(v)
		}
	}
	return ""
}

func boolFromEnv(v string) bool {
	switch strings.ToLower(strings.TrimSpace(v)) {
	case "1", "true", "yes", "on":
		return true
	default:
		return false
	}
}

func stringPtrOrNil(v types.String) *string {
	if v.IsNull() || v.IsUnknown() || v.ValueString() == "" {
		return nil
	}
	s := v.ValueString()
	return &s
}

func boolPtrOrNil(v types.Bool) *bool {
	if v.IsNull() || v.IsUnknown() {
		return nil
	}
	b := v.ValueBool()
	return &b
}

func optionalString(v *string) types.String {
	if v == nil || *v == "" {
		return types.StringNull()
	}
	return types.StringValue(*v)
}

func optionalBool(v *bool, fallback bool) types.Bool {
	if v == nil {
		return types.BoolValue(fallback)
	}
	return types.BoolValue(*v)
}
