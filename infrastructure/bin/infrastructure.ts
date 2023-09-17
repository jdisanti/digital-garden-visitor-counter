#!/usr/bin/env node
import "source-map-support/register";
import * as cdk from "aws-cdk-lib";
import { InfrastructureStack } from "../lib/infrastructure-stack";

new InfrastructureStack(new cdk.App(), "digital-garden-visitor-counter", {});
