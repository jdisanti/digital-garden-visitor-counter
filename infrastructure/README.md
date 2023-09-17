# Deployment infrastructure for the digital-garden-visitor-counter

This CDK project builds and deploys the digital-garden-visitor-counter Lambda
to an AWS account, and also creates the necessary DynamoDB table and permissions.

## Useful commands

Note: Use the Justfile in the repository root for building and synthesizing the Lambda.

* `npm run build`: compile typescript to js
* `npm run watch`: watch for changes and compile
* `npm run test`: perform the jest unit tests
* `npx cdk deploy`: deploy this stack to your default AWS account/region
* `npx cdk diff`: compare deployed stack with current state
* `npx cdk synth`: emits the synthesized CloudFormation template
