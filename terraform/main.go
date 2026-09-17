package main

import (
	"context"
	"flag"
	"log"

	"github.com/hashicorp/terraform-plugin-framework/providerserver"
	"github.com/pertisktech/pertisk-proxy/terraform/internal/provider"
)

var version = "0.1.1"

func main() {
	var debug bool
	flag.BoolVar(&debug, "debug", false, "run the provider with debugger support")
	flag.Parse()

	err := providerserver.Serve(context.Background(), provider.New(version), providerserver.ServeOpts{
		Address: "registry.terraform.io/pertisktech/pertisk-proxy",
		Debug:   debug,
	})
	if err != nil {
		log.Fatal(err.Error())
	}
}
