import { onRemotePending, remoteSetApproval } from './api.js';
import { showConfirm } from './modal.js';
import { closeQrOverlay } from './settings-ui.js';
import { t } from './i18n.js';

let chain = Promise.resolve();

function enqueue(device) {
  chain = chain
    .then(async () => {
      // The QR overlay sits above every modal: close it or the approval
      // dialog would be hidden underneath.
      closeQrOverlay();
      const ok = await showConfirm(
        t('remote.approve.body', {
          device: device.device || t('settings.remote.device'),
          ip: device.ip || '',
        }),
        {
          title: t('remote.approve.title'),
          variant: 'info',
          okLabel: t('remote.approve.ok'),
          cancelLabel: t('remote.approve.deny'),
        },
      );
      await remoteSetApproval(device.id, ok);
    })
    .catch(() => {});
}

onRemotePending(device => {
  if (device && device.id != null) enqueue(device);
}).catch(() => {});
