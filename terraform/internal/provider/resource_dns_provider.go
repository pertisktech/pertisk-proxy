package provider

import (
	"context"

	"github.com/hashicorp/terraform-plugin-framework/diag"
	"github.com/hashicorp/terraform-plugin-framework/path"
	"github.com/hashicorp/terraform-plugin-framework/resource"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema/planmodifier"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema/stringplanmodifier"
	"github.com/hashicorp/terraform-plugin-framework/types"

	"github.com/pertisktech/pertisk-proxy/terraform/internal/client"
)

var (
	_ resource.Resource                = &dnsProviderResource{}
	_ resource.ResourceWithConfigure   = &dnsProviderResource{}
	_ resource.ResourceWithImportState = &dnsProviderResource{}
)

func NewDnsProviderResource() resource.Resource { return &dnsProviderResource{} }

type dnsProviderResource struct {
	api *client.Client
}

type dnsProviderModel struct {
	ID           types.String `tfsdk:"id"`
	Name         types.String `tfsdk:"name"`
	ProviderType types.String `tfsdk:"provider_type"`
	Credentials  types.Map    `tfsdk:"credentials"`
}

func (r *dnsProviderResource) Metadata(_ context.Context, req resource.MetadataRequest, resp *resource.MetadataResponse) {
	resp.TypeName = req.ProviderTypeName + "_dns_provider"
}

func (r *dnsProviderResource) Schema(_ context.Context, _ resource.SchemaRequest, resp *resource.SchemaResponse) {
	resp.Schema = schema.Schema{
		MarkdownDescription: "A DNS provider used for ACME DNS-01 challenges.",
		Attributes: map[string]schema.Attribute{
			"id": schema.StringAttribute{
				Computed: true,
				PlanModifiers: []planmodifier.String{
					stringplanmodifier.UseStateForUnknown(),
				},
			},
			"name": schema.StringAttribute{
				Required:            true,
				MarkdownDescription: "Display name (referenced by TLS ACME config).",
			},
			"provider_type": schema.StringAttribute{
				Required:            true,
				MarkdownDescription: "`cloudflare`, `digitalocean`, `route53`, `duckdns`, or `hetzner`.",
			},
			"credentials": schema.MapAttribute{
				Optional:            true,
				Sensitive:           true,
				ElementType:         types.StringType,
				MarkdownDescription: "Provider-specific credentials map.",
			},
		},
	}
}

func (r *dnsProviderResource) Configure(_ context.Context, req resource.ConfigureRequest, resp *resource.ConfigureResponse) {
	r.api = configureClient(req.ProviderData, &resp.Diagnostics)
}

func (r *dnsProviderResource) Create(ctx context.Context, req resource.CreateRequest, resp *resource.CreateResponse) {
	var plan dnsProviderModel
	resp.Diagnostics.Append(req.Plan.Get(ctx, &plan)...)
	if resp.Diagnostics.HasError() {
		return
	}

	in, diags := dnsProviderInput(ctx, plan)
	resp.Diagnostics.Append(diags...)
	if resp.Diagnostics.HasError() {
		return
	}

	id, err := r.api.CreateDnsProvider(ctx, in)
	if err != nil {
		resp.Diagnostics.AddError("Create DNS provider failed", err.Error())
		return
	}
	plan.ID = types.StringValue(id)
	resp.Diagnostics.Append(resp.State.Set(ctx, &plan)...)
}

func (r *dnsProviderResource) Read(ctx context.Context, req resource.ReadRequest, resp *resource.ReadResponse) {
	var state dnsProviderModel
	resp.Diagnostics.Append(req.State.Get(ctx, &state)...)
	if resp.Diagnostics.HasError() {
		return
	}

	got, err := r.api.GetDnsProvider(ctx, state.ID.ValueString())
	if err != nil {
		resp.Diagnostics.AddError("Read DNS provider failed", err.Error())
		return
	}
	state.Name = types.StringValue(got.Name)
	state.ProviderType = types.StringValue(got.ProviderType)
	if len(got.Credentials) > 0 {
		m, diags := types.MapValueFrom(ctx, types.StringType, got.Credentials)
		resp.Diagnostics.Append(diags...)
		if resp.Diagnostics.HasError() {
			return
		}
		state.Credentials = m
	}
	resp.Diagnostics.Append(resp.State.Set(ctx, &state)...)
}

func (r *dnsProviderResource) Update(ctx context.Context, req resource.UpdateRequest, resp *resource.UpdateResponse) {
	var plan dnsProviderModel
	resp.Diagnostics.Append(req.Plan.Get(ctx, &plan)...)
	if resp.Diagnostics.HasError() {
		return
	}

	in, diags := dnsProviderInput(ctx, plan)
	resp.Diagnostics.Append(diags...)
	if resp.Diagnostics.HasError() {
		return
	}

	if err := r.api.UpdateDnsProvider(ctx, plan.ID.ValueString(), in); err != nil {
		resp.Diagnostics.AddError("Update DNS provider failed", err.Error())
		return
	}
	resp.Diagnostics.Append(resp.State.Set(ctx, &plan)...)
}

func (r *dnsProviderResource) Delete(ctx context.Context, req resource.DeleteRequest, resp *resource.DeleteResponse) {
	var state dnsProviderModel
	resp.Diagnostics.Append(req.State.Get(ctx, &state)...)
	if resp.Diagnostics.HasError() {
		return
	}
	if err := r.api.DeleteDnsProvider(ctx, state.ID.ValueString()); err != nil {
		resp.Diagnostics.AddError("Delete DNS provider failed", err.Error())
		return
	}
}

func (r *dnsProviderResource) ImportState(ctx context.Context, req resource.ImportStateRequest, resp *resource.ImportStateResponse) {
	resource.ImportStatePassthroughID(ctx, path.Root("id"), req, resp)
}

func dnsProviderInput(ctx context.Context, plan dnsProviderModel) (client.DnsProviderInput, diag.Diagnostics) {
	var diags diag.Diagnostics
	in := client.DnsProviderInput{
		Name:         plan.Name.ValueString(),
		ProviderType: plan.ProviderType.ValueString(),
	}
	if !plan.Credentials.IsNull() && !plan.Credentials.IsUnknown() {
		creds := map[string]string{}
		diags.Append(plan.Credentials.ElementsAs(ctx, &creds, false)...)
		in.Credentials = creds
	}
	return in, diags
}
