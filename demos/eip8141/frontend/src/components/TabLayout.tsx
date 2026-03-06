import { useState } from 'react';
import type { StoredCredential } from '../lib/passkey';
import SimpleSend from './SimpleSend';
import SponsoredSend from './SponsoredSend';
import BatchOps from './BatchOps';
import DeployExecute from './DeployExecute';

const TABS = [
  { id: 'simple', label: 'Simple Send' },
  { id: 'sponsored', label: 'Sponsored' },
  { id: 'batch', label: 'Batch' },
  { id: 'deploy', label: 'Deploy + Execute' },
] as const;

type TabId = (typeof TABS)[number]['id'];

interface Props {
  credential: StoredCredential;
}

export default function TabLayout({ credential }: Props) {
  const [active, setActive] = useState<TabId>('simple');

  return (
    <div>
      <div className="flex border-b border-zinc-800 mb-6">
        {TABS.map(tab => (
          <button
            key={tab.id}
            onClick={() => setActive(tab.id)}
            className={`px-4 py-2.5 text-sm font-medium transition-colors cursor-pointer ${
              active === tab.id
                ? 'text-indigo-400 border-b-2 border-indigo-400 -mb-px'
                : 'text-zinc-500 hover:text-zinc-300'
            }`}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {active === 'simple' && <SimpleSend credential={credential} />}
      {active === 'sponsored' && <SponsoredSend credential={credential} />}
      {active === 'batch' && <BatchOps credential={credential} />}
      {active === 'deploy' && <DeployExecute credential={credential} />}
    </div>
  );
}
