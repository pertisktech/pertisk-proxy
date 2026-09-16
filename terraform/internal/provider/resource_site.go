package provider

import (
	"context"
	"fmt"

	"github.com/hashicorp/terraform-plugin-framework/path"
	"github.com/hashicorp/terraform-plugin-framework/resource"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema/booldefault"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema/planmodifier"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema/stringdefault"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema/stringplanmodifier"
	"github.com/hashicorp/terraform-plugin-framework/types"

	"github.com/pertisktech/pertisk-proxy/terraform/internal/client"
)

var (
	_ resource.Resource                = &siteResource{}
	_ resource.ResourceWithConfigure   = &siteResource{}
	_ resource.ResourceWithImportState = &siteResource{}
)

func NewSiteResource() resource.Resource { return &siteResource{} }

type siteResource struct {
	api *client.Client
}

type siteRouteModel struct {
	Path     types.String `tfsdk:"path"`
	PathType types.String `tfsdk:"path_type"`
	Rewrite  types.String `tfsdk:"rewrite"`
	Upstream types.String `tfsdk:"upstream"`
}

type siteModel struct {
	ID              types.String     `tfsdk:"id"`
	Host            types.String     `tfsdk:"host"`
	Backend         types.String     `tfsdk:"backend"`
	BackendUpstream types.String     `tfsdk:"backend_upstream"`
	Routes          []siteRouteModel `tfsdk:"routes"`
	ForwardClientIP types.Bool       `tfsdk:"forward_client_ip"`
	AccessListID    types.String     `tfsdk:"access_list_id"`
	WafPolicyID     types.String     `tfsdk:"waf_policy_id"`
}

func (r *siteResource) Metadata(_ context.Context, req resource.MetadataRequest, resp *resource.MetadataResponse) {
	resp.TypeName = req.ProviderTypeName + "_site"
}

func (r *siteResource) Schema(_ context.Context, _ resource.SchemaRequest, resp *resource.SchemaResponse) {
	resp.Schema = schema.Schema{
		MarkdownDescription: "A reverse-proxy site managed via GET/PUT `/api/config` (keyed by `host`). Concurrent UI/Terraform edits can race; use one writer per proxy.",
		Attributes: map[string]schema.Attribute{
			"id": schema.StringAttribute{
				Computed:            true,
				MarkdownDescription: "Same as `host`.",
				PlanModifiers: []planmodifier.String{
					stringplanmodifier.UseStateForUnknown(),
				},
			},
			"host": schema.StringAttribute{
				Required:            true,
				MarkdownDescription: "Hostname this site matches (SNI / Host header).",
				PlanModifiers: []planmodifier.String{
					stringplanmodifier.RequiresReplace(),
				},
			},
			"backend": schema.StringAttribute{
				Required:            true,
				MarkdownDescription: "Backend name referenced by the site.",
			},
			"backend_upstream": schema.StringAttribute{
				Optional:            true,
				MarkdownDescription: "When set, ensure a backend with `backend` name exists pointing at this upstream URL (e.g. `http://127.0.0.1:8080`).",
			},
			"forward_client_ip": schema.BoolAttribute{
				Optional:            true,
				Computed:            true,
				Default:             booldefault.StaticBool(true),
				MarkdownDescription: "Inject `X-Real-IP` / `X-Forwarded-For`.",
			},
			"access_list_id": schema.StringAttribute{
				Optional:            true,
				MarkdownDescription: "Named access list id.",
			},
			"waf_policy_id": schema.StringAttribute{
				Optional:            true,
				MarkdownDescription: "Named WAF policy id.",
			},
		},
		Blocks: map[string]schema.Block{
			"routes": schema.ListNestedBlock{
				MarkdownDescription: "Path routes for the site. At least one is required.",
				NestedObject: schema.NestedBlockObject{
					Attributes: map[string]schema.Attribute{
						"path": schema.StringAttribute{
							Required: true,
						},
						"path_type": schema.StringAttribute{
							Optional:            true,
							Computed:            true,
							Default:             stringdefault.StaticString("Prefix"),
							MarkdownDescription: "`Prefix`, `Exact`, or `Regex`.",
						},
						"rewrite": schema.StringAttribute{
							Optional: true,
						},
						"upstream": schema.StringAttribute{
							Optional:            true,
							MarkdownDescription: "Optional per-route upstream override URL.",
						},
					},
				},
			},
		},
	}
}

func (r *siteResource) Configure(_ context.Context, req resource.ConfigureRequest, resp *resource.ConfigureResponse) {
	r.api = configureClient(req.ProviderData, &resp.Diagnostics)
}

func (r *siteResource) Create(ctx context.Context, req resource.CreateRequest, resp *resource.CreateResponse) {
	var plan siteModel
	resp.Diagnostics.Append(req.Plan.Get(ctx, &plan)...)
	if resp.Diagnostics.HasError() {
		return
	}
	if err := r.apply(ctx, plan); err != nil {
		resp.Diagnostics.AddError("Create site failed", err.Error())
		return
	}
	plan.ID = plan.Host
	resp.Diagnostics.Append(resp.State.Set(ctx, &plan)...)
}

func (r *siteResource) Read(ctx context.Context, req resource.ReadRequest, resp *resource.ReadResponse) {
	var state siteModel
	resp.Diagnostics.Append(req.State.Get(ctx, &state)...)
	if resp.Diagnostics.HasError() {
		return
	}

	cfg, err := r.api.GetConfig(ctx)
	if err != nil {
		resp.Diagnostics.AddError("Read config failed", err.Error())
		return
	}
	site := client.FindSite(cfg, state.Host.ValueString())
	if site == nil {
		resp.State.RemoveResource(ctx)
		return
	}

	state.ID = types.StringValue(site.Host)
	state.Host = types.StringValue(site.Host)
	state.Backend = types.StringValue(site.Backend)
	state.ForwardClientIP = optionalBool(site.ForwardClientIP, true)
	state.AccessListID = optionalString(site.AccessListID)
	state.WafPolicyID = optionalString(site.WafPolicyID)
	state.Routes = make([]siteRouteModel, 0, len(site.Routes))
	for _, rt := range site.Routes {
		pathType := rt.PathType
		if pathType == "" {
			pathType = "Prefix"
		}
		state.Routes = append(state.Routes, siteRouteModel{
			Path:     types.StringValue(rt.Path),
			PathType: types.StringValue(pathType),
			Rewrite:  optionalString(rt.Rewrite),
			Upstream: optionalString(rt.Upstream),
		})
	}

	if !state.BackendUpstream.IsNull() && !state.BackendUpstream.IsUnknown() {
		for _, b := range cfg.Backends {
			if b.Name == site.Backend && len(b.Upstreams) > 0 {
				state.BackendUpstream = types.StringValue(b.Upstreams[0].Addr)
				break
			}
		}
	}

	resp.Diagnostics.Append(resp.State.Set(ctx, &state)...)
}

func (r *siteResource) Update(ctx context.Context, req resource.UpdateRequest, resp *resource.UpdateResponse) {
	var plan siteModel
	resp.Diagnostics.Append(req.Plan.Get(ctx, &plan)...)
	if resp.Diagnostics.HasError() {
		return
	}
	if err := r.apply(ctx, plan); err != nil {
		resp.Diagnostics.AddError("Update site failed", err.Error())
		return
	}
	plan.ID = plan.Host
	resp.Diagnostics.Append(resp.State.Set(ctx, &plan)...)
}

func (r *siteResource) Delete(ctx context.Context, req resource.DeleteRequest, resp *resource.DeleteResponse) {
	var state siteModel
	resp.Diagnostics.Append(req.State.Get(ctx, &state)...)
	if resp.Diagnostics.HasError() {
		return
	}

	cfg, err := r.api.GetConfig(ctx)
	if err != nil {
		resp.Diagnostics.AddError("Read config failed", err.Error())
		return
	}
	if !client.RemoveSite(cfg, state.Host.ValueString()) {
		return
	}
	if _, err := r.api.PutConfig(ctx, cfg); err != nil {
		resp.Diagnostics.AddError("Delete site failed", err.Error())
		return
	}
}

func (r *siteResource) ImportState(ctx context.Context, req resource.ImportStateRequest, resp *resource.ImportStateResponse) {
	resource.ImportStatePassthroughID(ctx, path.Root("host"), req, resp)
	resp.Diagnostics.Append(resp.State.SetAttribute(ctx, path.Root("id"), req.ID)...)
}

func (r *siteResource) apply(ctx context.Context, plan siteModel) error {
	if r.api == nil {
		return fmt.Errorf("provider not configured")
	}
	if len(plan.Routes) == 0 {
		return fmt.Errorf("at least one routes block is required")
	}

	cfg, err := r.api.GetConfig(ctx)
	if err != nil {
		return err
	}

	site := client.Site{
		Host:            plan.Host.ValueString(),
		Backend:         plan.Backend.ValueString(),
		ForwardClientIP: boolPtrOrNil(plan.ForwardClientIP),
		AccessListID:    stringPtrOrNil(plan.AccessListID),
		WafPolicyID:     stringPtrOrNil(plan.WafPolicyID),
		Routes:          make([]client.PathRewrite, 0, len(plan.Routes)),
	}
	for _, rt := range plan.Routes {
		pr := client.PathRewrite{
			Path:     rt.Path.ValueString(),
			PathType: rt.PathType.ValueString(),
			Rewrite:  stringPtrOrNil(rt.Rewrite),
			Upstream: stringPtrOrNil(rt.Upstream),
		}
		site.Routes = append(site.Routes, pr)
	}

	client.UpsertSite(cfg, site, plan.BackendUpstream.ValueString())
	_, err = r.api.PutConfig(ctx, cfg)
	return err
}
