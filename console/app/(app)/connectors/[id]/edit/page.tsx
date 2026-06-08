import { updateConnectorAction } from '@/app/(app)/connectors/actions';
import { ConnectorForm } from '@/components/ConnectorForm';
import { ErrorState, PageHeader } from '@/components/States';
import { canManage } from '@/lib/rbac';
import { getAdminClient } from '@/lib/session';

export const dynamic = 'force-dynamic';

/** Edit-connector page (Admin only). Prefills the form from the stored config. */
export default async function EditConnectorPage({ params }: { params: Promise<{ id: string }> }) {
    const { id } = await params;
    const client = await getAdminClient();
    if (!client) return <ErrorState error="No admin client (signed out)" />;

    let role;
    let existing;
    try {
        [role, existing] = await Promise.all([client.me().then((p) => p.role), client.getConnector(id)]);
    } catch (err) {
        return (
            <div>
                <PageHeader title="Edit connector" />
                <ErrorState error={err} />
            </div>
        );
    }

    if (!canManage(role)) {
        return (
            <div>
                <PageHeader title="Edit connector" />
                <ErrorState error="Editing connectors requires the Admin role." />
            </div>
        );
    }

    // Bind the connector id so the form sees the `(prev, form)` action shape.
    const action = updateConnectorAction.bind(null, id);

    return (
        <div>
            <PageHeader title="Edit connector" subtitle={existing.name} />
            <ConnectorForm action={action} existing={existing} submitLabel="Save changes" />
        </div>
    );
}
