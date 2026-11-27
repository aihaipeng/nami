import type { ViewProps } from '../App';
import { Clipboard } from '../clipboard/Clipboard';

export function RecordsWorkspace(props: ViewProps) {
  const { snapshot } = props;
  const visible = snapshot.window.clipboardVisible;
  return <div className="records-workspace" data-record-view="clipboard">
    <div className="record-view" inert={!visible}>
      <Clipboard {...props} active={visible} />
    </div>
  </div>;
}
