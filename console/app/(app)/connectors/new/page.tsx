import { createConnectorAction } from '@/app/(app)/connectors/actions';
import { ConnectorForm } from '@/components/ConnectorForm';
import { ErrorState, PageHeader } from '@/components/States';
import { canManage } from '@/lib/rbac';
import { getAdminClient } from '@/lib/session';

export const dynamic = 'force-dynamic';

/** Create-connector page (Admin only). */
export default async function NewConnectorPage() {
    const client = await getAdminClient();
    if (!client) return <ErrorState error="No admin client (signed out)" />;

    // Gate the page on Admin so a Curator never reaches the form (the server
    // re-enforces this on POST regardless).
    let role;
    try {
        role = (await client.me()).role;
    } catch (err) {
        return <ErrorState error={err} />;
    }
    if (!canManage(role)) {
        return (
            <div>
                <PageHeader title="New connector" />
                <ErrorState error="Creating connectors requires the Admin role." />
            </div>
        );
    }

    return (
        <div>
            <PageHeader title="New connector" subtitle="Configure a source for the indexing loop." />
            <ConnectorForm action={createConnectorAction} submitLabel="Create connector" />
        </div>
    );
}
