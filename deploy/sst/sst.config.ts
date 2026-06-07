/* eslint-disable @typescript-eslint/no-explicit-any -- SST config uses ambient $-globals */
/// <reference path="./.sst/platform/config.d.ts" />

/**
 * SST v4 app — the AWS-serverless deploy path for `smooth-operator-agent`.
 *
 * Provisions the API Gateway WebSocket + the Rust Lambda (the
 * `smooai-smooth-operator-agent-lambda` crate) + the DynamoDB single table + an
 * S3 blob bucket, wires the S3 Vectors index, and links everything to the
 * function's environment. The Lambda serves the schema-driven protocol over API
 * Gateway WebSocket, posting events back via the API Gateway Management API.
 *
 * NEVER deploy locally — see `README.md`. CI owns deploys. Verification here is
 * `npx tsc --noEmit` + synth, not `sst deploy`.
 *
 * ## The Rust-Lambda build seam
 * SST has no native Rust builder, so the Lambda bootstrap is built out-of-band
 * with `cargo lambda` (see `README.md`) into
 * `../../rust/target/lambda/smooai-smooth-operator-agent-lambda/`, and the
 * `Function` points at that prebuilt artifact directory with the
 * `provided.al2023` custom runtime on `arm64`. The `ARTIFACT_DIR` constant is
 * the single place that path is declared.
 *
 * ## The S3 Vectors gap
 * SST v4 ships no native S3 Vectors component (the service went GA 2025-12).
 * The vector bucket + per-org index are declared with the raw
 * `aws.s3vectors.*` Pulumi resources when available, with a documented
 * CloudFormation/aws-cli fallback in `README.md`. The Lambda reads the bucket
 * name + index prefix from env and uses its `s3-vectors` adapter feature.
 */

const ARTIFACT_DIR = '../../rust/target/lambda/smooai-smooth-operator-agent-lambda';

// Match SST's Lambda timeout/memory literal-union types so the per-route helper
// stays type-safe without importing SST's internal `Duration`/`Size` modules.
type RouteTimeout = `${number} second` | `${number} seconds` | `${number} minute` | `${number} minutes`;
type RouteMemory = `${number} MB` | `${number} GB`;

export default $config({
    app(input) {
        return {
            name: 'smooth-operator-agent',
            removal: input?.stage === 'production' ? 'retain' : 'remove',
            protect: ['production'].includes(input?.stage ?? ''),
            home: 'aws',
            providers: {
                aws: {
                    region: (process.env.AWS_REGION as any) ?? 'us-east-1',
                },
            },
        };
    },

    async run() {
        // ── DynamoDB single table ───────────────────────────────────────────
        // Mirrors the adapter's key design (rust/adapters/dynamodb/src/keys.rs):
        // overloaded `pk`/`sk` primary key + one all-projecting GSI `gsi1` over
        // `gsi1pk`/`gsi1sk`. PAY_PER_REQUEST is SST's default for `Dynamo`.
        // `ttl` powers the $connect/$disconnect connection-registry rows.
        const table = new sst.aws.Dynamo('SmoothAgentTable', {
            fields: {
                pk: 'string',
                sk: 'string',
                gsi1pk: 'string',
                gsi1sk: 'string',
            },
            primaryIndex: { hashKey: 'pk', rangeKey: 'sk' },
            globalIndexes: {
                gsi1: { hashKey: 'gsi1pk', rangeKey: 'gsi1sk' },
            },
            ttl: 'ttl',
        });

        // ── S3 blob bucket ──────────────────────────────────────────────────
        // General-purpose blob storage (attachments / large payloads). The
        // vector data lives in a *separate* S3 Vectors bucket below.
        const blobs = new sst.aws.Bucket('SmoothAgentBlobs');

        // ── S3 Vectors index (the gap) ──────────────────────────────────────
        // SST v4 has no native S3 Vectors component. When the AWS Pulumi
        // provider exposes `s3vectors`, declare the vector bucket + per-org
        // index here. Until then this is provisioned out-of-band (see
        // README.md) and the Lambda is pointed at it via env. We declare the
        // *intended* names so the rest of the wiring is stable.
        const vectorBucketName = $interpolate`smooth-agent-vectors-${$app.stage}`;
        const vectorIndexPrefix = 'smooth-agent-knowledge';

        // Optional: when the raw provider is available, uncomment to manage the
        // S3 Vectors bucket + index as first-class resources. Kept commented so
        // `tsc`/synth don't fail on provider versions that predate s3vectors.
        //
        // const vectorBucket = new aws.s3vectors.VectorBucket('SmoothAgentVectorBucket', {
        //     vectorBucketName,
        // });
        // const vectorIndex = new aws.s3vectors.Index('SmoothAgentVectorIndex', {
        //     vectorBucketName: vectorBucket.vectorBucketName,
        //     indexName: $interpolate`${vectorIndexPrefix}-default`,
        //     dataType: 'float32',
        //     dimension: 1024,
        //     distanceMetric: 'cosine',
        // });

        // ── Gateway key secret ──────────────────────────────────────────────
        // The smooai monorepo standard is @smooai/config, but this standalone
        // OSS repo uses an sst.Secret placeholder (see README.md). Set with
        // `npx sst secret set SmoothAgentGatewayKey <key> --stage <x>`.
        const gatewayKey = new sst.Secret('SmoothAgentGatewayKey');
        const gatewayUrl = new sst.Secret('SmoothAgentGatewayUrl', 'https://llm.smoo.ai/v1');
        const model = new sst.Secret('SmoothAgentModel', 'claude-haiku-4-5');

        // Common environment for the Rust Lambda. The adapter reads
        // SMOOTH_AGENT_DDB_TABLE; the rest map 1:1 to `LambdaConfig::from_env`.
        const environment = {
            SMOOTH_AGENT_DDB_TABLE: table.name,
            SMOOAI_GATEWAY_URL: gatewayUrl.value,
            SMOOAI_GATEWAY_KEY: gatewayKey.value,
            SMOOTH_AGENT_MODEL: model.value,
            SMOOTH_AGENT_ORG_ID: 'default',
            SMOOTH_AGENT_VECTOR_BUCKET: vectorBucketName,
            SMOOTH_AGENT_VECTOR_INDEX_PREFIX: vectorIndexPrefix,
            SMOOTH_AGENT_MAX_ITERATIONS: '6',
            SMOOTH_AGENT_MAX_TOKENS: '512',
        };

        // ── API Gateway WebSocket ───────────────────────────────────────────
        const api = new sst.aws.ApiGatewayWebSocket('SmoothAgentApi');

        // One Rust Lambda serves every route — `requestContext.routeKey` selects
        // the behavior inside the binary (main.rs). All routes share the same
        // prebuilt bootstrap artifact + env + links. `link` grants IAM + injects
        // the resource names; the Management API permission for post-back is
        // granted via the explicit `permissions` block below.
        const route = (routeKey: string, timeout: RouteTimeout, memory: RouteMemory) =>
            api.route(routeKey, {
                // Prebuilt by `cargo lambda build --release --arm64` (README.md).
                handler: ARTIFACT_DIR,
                runtime: 'provided.al2023',
                architecture: 'arm64',
                timeout,
                memory,
                environment,
                link: [table, blobs, gatewayKey, gatewayUrl, model],
                permissions: [
                    // Post events back to the connected client.
                    { actions: ['execute-api:ManageConnections'], resources: ['*'] },
                    // S3 Vectors put/query for the knowledge backend.
                    {
                        actions: ['s3vectors:PutVectors', 's3vectors:QueryVectors', 's3vectors:GetVectors'],
                        resources: ['*'],
                    },
                ],
            });

        route('$connect', '30 seconds', '256 MB');
        route('$disconnect', '30 seconds', '256 MB');
        route('send_message', '5 minutes', '1024 MB');
        route('ping', '10 seconds', '256 MB');
        // The protocol's other actions arrive on $default (the SDK clients send
        // a JSON envelope with an `action` field), dispatched inside the binary.
        route('$default', '2 minutes', '512 MB');

        return {
            api: api.url,
            table: table.name,
            blobs: blobs.name,
            vectorBucket: vectorBucketName,
        };
    },
});
