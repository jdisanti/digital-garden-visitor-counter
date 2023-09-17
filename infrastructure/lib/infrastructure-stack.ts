// Digital garden visitor counter
// A simple visitor counter for digital gardens that runs as an AWS Lambda function.
// Copyright (C) 2023 John DiSanti.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

import {
    Stack,
    StackProps,
    Duration,
    CfnOutput,
    CfnResource,
    CfnParameter,
} from "aws-cdk-lib";
import { AttributeType, BillingMode, Table } from "aws-cdk-lib/aws-dynamodb";
import { ManagedPolicy, Role, ServicePrincipal } from "aws-cdk-lib/aws-iam";
import {
    Code,
    Runtime,
    Function,
    Architecture,
    FunctionUrl,
    FunctionUrlAuthType,
} from "aws-cdk-lib/aws-lambda";
import { RetentionDays } from "aws-cdk-lib/aws-logs";
import { Construct } from "constructs";

export class InfrastructureStack extends Stack {
    constructor(scope: Construct, id: string, props?: StackProps) {
        super(scope, id, props);

        const counterTable = new Table(this, "counter-table", {
            partitionKey: {
                name: "key",
                type: AttributeType.STRING,
            },
            billingMode: BillingMode.PAY_PER_REQUEST,
            tableName: "digital-garden-visitor-counter",
        });

        const executionRole = new Role(this, "counter-execution-role", {
            roleName: "digital-garden-visitor-counter-lambda",
            assumedBy: new ServicePrincipal("lambda.amazonaws.com"),
        });
        executionRole.addManagedPolicy(
            ManagedPolicy.fromAwsManagedPolicyName(
                "service-role/AWSLambdaBasicExecutionRole",
            ),
        );
        executionRole.addManagedPolicy(
            ManagedPolicy.fromAwsManagedPolicyName(
                "service-role/AWSLambdaVPCAccessExecutionRole",
            ),
        );
        counterTable.grantReadWriteData(executionRole);

        const allowedNamesParam = new CfnParameter(this, "allowed-names", {
            type: "String",
            description: "Comma-separated list of allowed counter names",
            default: "default,repo-readme",
        });
        const minWidthParam = new CfnParameter(this, "min-width", {
            type: "String",
            description: "Minimum width of the counter in digits",
            default: "5",
        });

        const counterLambda = new Function(this, "counter-lambda", {
            architecture: Architecture.ARM_64,
            code: Code.fromAsset("build/bootstrap/bootstrap.zip"),
            environment: {
                DGVC_ALLOWED_NAMES: allowedNamesParam.valueAsString,
                DGVC_MIN_WIDTH: minWidthParam.valueAsString,
                DGVC_TABLE_NAME: counterTable.tableName,
                RUST_BACKTRACE: "1",
            },
            functionName: "digital-garden-visitor-counter",
            handler: "not.used",
            logRetention: RetentionDays.ONE_WEEK,
            memorySize: 128,
            role: executionRole,
            runtime: Runtime.PROVIDED_AL2,
            timeout: Duration.seconds(1),
        });

        const counterUrl = new FunctionUrl(this, "counter-url", {
            authType: FunctionUrlAuthType.NONE,
            function: counterLambda,
        });

        const counterInvokePermission = new CfnResource(
            this,
            "counter-url-invoke-permission",
            {
                type: "AWS::Lambda::Permission",
                properties: {
                    Action: "lambda:InvokeFunctionUrl",
                    FunctionName: counterLambda.functionName,
                    Principal: "*",
                    FunctionUrlAuthType: "NONE",
                },
            },
        );

        new CfnOutput(this, "counter-url-output", {
            value: counterUrl.url,
        });
    }
}
