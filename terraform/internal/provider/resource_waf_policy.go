package provider

import (
	"context"
	"encoding/json"

	"github.com/hashicorp/terraform-plugin-framework/path"
	"github.com/hashicorp/terraform-plugin-framework/resource"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema/planmodifier"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema/stringdefault"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema/stringplanmodifier"
	"github.com/hashicorp/terraform-plugin-framework/types"

	"github.com/pertisktech/pertisk-proxy/terraform/internal/client"
)

var (
	_ resource.Resource                = &wafPolicyResource{}
	_ resource.ResourceWithConfigure   = &wafPolicyResource{}
	_ resource.ResourceWithImportState = &wafPolicyResource{}
)

func NewWafPolicyResource() resource.Resource { return &wafPolicyResource{} }

type wafPolicyResource struct {
	api *client.Client
}

type wafPolicyModel struct {
	ID           types.String `tfsdk:"id"`
	Name         types.String `tfsdk:"name"`
	Description  types.String `tfsdk:"description"`
	SecurityJSON types.String `tfsdk:"security_json"`
}

func (r *wafPolicyResource) Metadata(_ context.Context, req resource.MetadataRequest, resp *resource.MetadataResponse) {
	resp.TypeName = req.ProviderTypeName + "_waf_policy"
}

func (r *wafPolicyResource) Schema(_ context.Context, _ resource.SchemaRequest, resp *resource.SchemaResponse) {
	resp.Schema = schema.Schema{
		MarkdownDescription: "A named WAF / bot / captcha policy.",
		Attributes: map[string]schema.Attribute{
			"id": schema.StringAttribute{
				Computed: true,
				PlanModifiers: []planmodifier.String{
					stringplanmodifier.UseStateForUnknown(),
				},
			},
			"name": schema.StringAttribute{
				Required: true,
			},
			"description": schema.StringAttribute{
				Optional: true,
			},
			"security_json": schema.StringAttribute{
				Optional:            true,
				Computed:            true,
				Default:             stringdefault.StaticString("{}"),
				MarkdownDescription: "JSON object matching the management API `security` field (`waf`, `bot`, `captcha`).",
			},
		},
	}
}

func (r *wafPolicyResource) Configure(_ context.Context, req resource.ConfigureRequest, resp *resource.ConfigureResponse) {
	r.api = configureClient(req.ProviderData, &resp.Diagnostics)
}

func (r *wafPolicyResource) Create(ctx context.Context, req resource.CreateRequest, resp *resource.CreateResponse) {
	var plan wafPolicyModel
	resp.Diagnostics.Append(req.Plan.Get(ctx, &plan)...)
	if resp.Diagnostics.HasError() {
		return
	}
	sec, err := parseSecurityJSON(plan.SecurityJSON.ValueString())
	if err != nil {
		resp.Diagnostics.AddAttributeError(path.Root("security_json"), "Invalid security_json", err.Error())
		return
	}
	id, err := r.api.CreateWafPolicy(ctx, client.WafPolicyInput{
		Name:        plan.Name.ValueString(),
		Description: stringPtrOrNil(plan.Description),
		Security:    sec,
	})
	if err != nil {
		resp.Diagnostics.AddError("Create WAF policy failed", err.Error())
		return
	}
	plan.ID = types.StringValue(id)
	resp.Diagnostics.Append(resp.State.Set(ctx, &plan)...)
}

func (r *wafPolicyResource) Read(ctx context.Context, req resource.ReadRequest, resp *resource.ReadResponse) {
	var state wafPolicyModel
	resp.Diagnostics.Append(req.State.Get(ctx, &state)...)
	if resp.Diagnostics.HasError() {
		return
	}
	got, err := r.api.GetWafPolicy(ctx, state.ID.ValueString())
	if err != nil {
		resp.Diagnostics.AddError("Read WAF policy failed", err.Error())
		return
	}
	state.Name = types.StringValue(got.Name)
	state.Description = optionalString(got.Description)
	b, err := json.Marshal(got.Security)
	if err != nil {
		resp.Diagnostics.AddError("Encode security failed", err.Error())
		return
	}
	state.SecurityJSON = types.StringValue(string(b))
	resp.Diagnostics.Append(resp.State.Set(ctx, &state)...)
}

func (r *wafPolicyResource) Update(ctx context.Context, req resource.UpdateRequest, resp *resource.UpdateResponse) {
	var plan wafPolicyModel
	resp.Diagnostics.Append(req.Plan.Get(ctx, &plan)...)
	if resp.Diagnostics.HasError() {
		return
	}
	sec, err := parseSecurityJSON(plan.SecurityJSON.ValueString())
	if err != nil {
		resp.Diagnostics.AddAttributeError(path.Root("security_json"), "Invalid security_json", err.Error())
		return
	}
	if err := r.api.UpdateWafPolicy(ctx, plan.ID.ValueString(), client.WafPolicyInput{
		Name:        plan.Name.ValueString(),
		Description: stringPtrOrNil(plan.Description),
		Security:    sec,
	}); err != nil {
		resp.Diagnostics.AddError("Update WAF policy failed", err.Error())
		return
	}
	resp.Diagnostics.Append(resp.State.Set(ctx, &plan)...)
}

func (r *wafPolicyResource) Delete(ctx context.Context, req resource.DeleteRequest, resp *resource.DeleteResponse) {
	var state wafPolicyModel
	resp.Diagnostics.Append(req.State.Get(ctx, &state)...)
	if resp.Diagnostics.HasError() {
		return
	}
	if err := r.api.DeleteWafPolicy(ctx, state.ID.ValueString()); err != nil {
		resp.Diagnostics.AddError("Delete WAF policy failed", err.Error())
		return
	}
}

func (r *wafPolicyResource) ImportState(ctx context.Context, req resource.ImportStateRequest, resp *resource.ImportStateResponse) {
	resource.ImportStatePassthroughID(ctx, path.Root("id"), req, resp)
}

func parseSecurityJSON(raw string) (*client.Security, error) {
	if raw == "" {
		raw = "{}"
	}
	var sec client.Security
	if err := json.Unmarshal([]byte(raw), &sec); err != nil {
		return nil, err
	}
	return &sec, nil
}
