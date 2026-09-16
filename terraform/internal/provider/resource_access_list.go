package provider

import (
	"context"

	"github.com/hashicorp/terraform-plugin-framework/diag"
	"github.com/hashicorp/terraform-plugin-framework/path"
	"github.com/hashicorp/terraform-plugin-framework/resource"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema/booldefault"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema/planmodifier"
	"github.com/hashicorp/terraform-plugin-framework/resource/schema/stringplanmodifier"
	"github.com/hashicorp/terraform-plugin-framework/types"

	"github.com/pertisktech/pertisk-proxy/terraform/internal/client"
)

var (
	_ resource.Resource                = &accessListResource{}
	_ resource.ResourceWithConfigure   = &accessListResource{}
	_ resource.ResourceWithImportState = &accessListResource{}
)

func NewAccessListResource() resource.Resource { return &accessListResource{} }

type accessListResource struct {
	api *client.Client
}

type accessListModel struct {
	ID             types.String `tfsdk:"id"`
	Name           types.String `tfsdk:"name"`
	Description    types.String `tfsdk:"description"`
	Enabled        types.Bool   `tfsdk:"enabled"`
	AllowCountries types.List   `tfsdk:"allow_countries"`
	DenyCountries  types.List   `tfsdk:"deny_countries"`
	AllowASNs      types.List   `tfsdk:"allow_asns"`
	DenyASNs       types.List   `tfsdk:"deny_asns"`
}

func (r *accessListResource) Metadata(_ context.Context, req resource.MetadataRequest, resp *resource.MetadataResponse) {
	resp.TypeName = req.ProviderTypeName + "_access_list"
}

func (r *accessListResource) Schema(_ context.Context, _ resource.SchemaRequest, resp *resource.SchemaResponse) {
	resp.Schema = schema.Schema{
		MarkdownDescription: "A named GeoIP access control list.",
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
			"enabled": schema.BoolAttribute{
				Optional: true,
				Computed: true,
				Default:  booldefault.StaticBool(false),
			},
			"allow_countries": schema.ListAttribute{
				Optional:    true,
				ElementType: types.StringType,
			},
			"deny_countries": schema.ListAttribute{
				Optional:    true,
				ElementType: types.StringType,
			},
			"allow_asns": schema.ListAttribute{
				Optional:    true,
				ElementType: types.Int64Type,
			},
			"deny_asns": schema.ListAttribute{
				Optional:    true,
				ElementType: types.Int64Type,
			},
		},
	}
}

func (r *accessListResource) Configure(_ context.Context, req resource.ConfigureRequest, resp *resource.ConfigureResponse) {
	r.api = configureClient(req.ProviderData, &resp.Diagnostics)
}

func (r *accessListResource) Create(ctx context.Context, req resource.CreateRequest, resp *resource.CreateResponse) {
	var plan accessListModel
	resp.Diagnostics.Append(req.Plan.Get(ctx, &plan)...)
	if resp.Diagnostics.HasError() {
		return
	}
	in, diags := accessListInput(ctx, plan)
	resp.Diagnostics.Append(diags...)
	if resp.Diagnostics.HasError() {
		return
	}
	id, err := r.api.CreateAccessList(ctx, in)
	if err != nil {
		resp.Diagnostics.AddError("Create access list failed", err.Error())
		return
	}
	plan.ID = types.StringValue(id)
	resp.Diagnostics.Append(resp.State.Set(ctx, &plan)...)
}

func (r *accessListResource) Read(ctx context.Context, req resource.ReadRequest, resp *resource.ReadResponse) {
	var state accessListModel
	resp.Diagnostics.Append(req.State.Get(ctx, &state)...)
	if resp.Diagnostics.HasError() {
		return
	}
	got, err := r.api.GetAccessList(ctx, state.ID.ValueString())
	if err != nil {
		resp.Diagnostics.AddError("Read access list failed", err.Error())
		return
	}
	state.Name = types.StringValue(got.Name)
	state.Description = optionalString(got.Description)
	state.Enabled = types.BoolValue(got.GeoIP.Enabled)
	state.AllowCountries = stringListOrNull(ctx, got.GeoIP.AllowCountries, &resp.Diagnostics)
	state.DenyCountries = stringListOrNull(ctx, got.GeoIP.DenyCountries, &resp.Diagnostics)
	state.AllowASNs = int64ListOrNull(ctx, got.GeoIP.AllowASNs, &resp.Diagnostics)
	state.DenyASNs = int64ListOrNull(ctx, got.GeoIP.DenyASNs, &resp.Diagnostics)
	if resp.Diagnostics.HasError() {
		return
	}
	resp.Diagnostics.Append(resp.State.Set(ctx, &state)...)
}

func (r *accessListResource) Update(ctx context.Context, req resource.UpdateRequest, resp *resource.UpdateResponse) {
	var plan accessListModel
	resp.Diagnostics.Append(req.Plan.Get(ctx, &plan)...)
	if resp.Diagnostics.HasError() {
		return
	}
	in, diags := accessListInput(ctx, plan)
	resp.Diagnostics.Append(diags...)
	if resp.Diagnostics.HasError() {
		return
	}
	if err := r.api.UpdateAccessList(ctx, plan.ID.ValueString(), in); err != nil {
		resp.Diagnostics.AddError("Update access list failed", err.Error())
		return
	}
	resp.Diagnostics.Append(resp.State.Set(ctx, &plan)...)
}

func (r *accessListResource) Delete(ctx context.Context, req resource.DeleteRequest, resp *resource.DeleteResponse) {
	var state accessListModel
	resp.Diagnostics.Append(req.State.Get(ctx, &state)...)
	if resp.Diagnostics.HasError() {
		return
	}
	if err := r.api.DeleteAccessList(ctx, state.ID.ValueString()); err != nil {
		resp.Diagnostics.AddError("Delete access list failed", err.Error())
		return
	}
}

func (r *accessListResource) ImportState(ctx context.Context, req resource.ImportStateRequest, resp *resource.ImportStateResponse) {
	resource.ImportStatePassthroughID(ctx, path.Root("id"), req, resp)
}

func accessListInput(ctx context.Context, plan accessListModel) (client.AccessListInput, diag.Diagnostics) {
	var diags diag.Diagnostics
	geo := &client.GeoIP{Enabled: plan.Enabled.ValueBool()}
	if !plan.AllowCountries.IsNull() && !plan.AllowCountries.IsUnknown() {
		var v []string
		diags.Append(plan.AllowCountries.ElementsAs(ctx, &v, false)...)
		geo.AllowCountries = v
	}
	if !plan.DenyCountries.IsNull() && !plan.DenyCountries.IsUnknown() {
		var v []string
		diags.Append(plan.DenyCountries.ElementsAs(ctx, &v, false)...)
		geo.DenyCountries = v
	}
	if !plan.AllowASNs.IsNull() && !plan.AllowASNs.IsUnknown() {
		var v []int64
		diags.Append(plan.AllowASNs.ElementsAs(ctx, &v, false)...)
		geo.AllowASNs = v
	}
	if !plan.DenyASNs.IsNull() && !plan.DenyASNs.IsUnknown() {
		var v []int64
		diags.Append(plan.DenyASNs.ElementsAs(ctx, &v, false)...)
		geo.DenyASNs = v
	}
	return client.AccessListInput{
		Name:        plan.Name.ValueString(),
		Description: stringPtrOrNil(plan.Description),
		GeoIP:       geo,
	}, diags
}

func stringListOrNull(ctx context.Context, values []string, diags *diag.Diagnostics) types.List {
	if len(values) == 0 {
		return types.ListNull(types.StringType)
	}
	v, d := types.ListValueFrom(ctx, types.StringType, values)
	diags.Append(d...)
	return v
}

func int64ListOrNull(ctx context.Context, values []int64, diags *diag.Diagnostics) types.List {
	if len(values) == 0 {
		return types.ListNull(types.Int64Type)
	}
	v, d := types.ListValueFrom(ctx, types.Int64Type, values)
	diags.Append(d...)
	return v
}
