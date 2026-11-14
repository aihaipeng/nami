type IconName = 'history' | 'pin' | 'close' | 'search' | 'copy' | 'settings' | 'clipboard' | 'translate' | 'arrow';

const paths: Record<IconName, string> = {
  history: 'M3 7v5h5 M4 11a8 8 0 1 1 1.8 7 M12 7v5l3 2',
  pin: 'M9 3h6 M10 3v6l-4 4v2h12v-2l-4-4V3 M12 15v6',
  close: 'm6 6 12 12 M18 6 6 18',
  search: 'M16 10a6 6 0 1 1-12 0 6 6 0 0 1 12 0 M14.5 14.5 21 21',
  copy: 'M8 8h12v13H8z M16 8V3H3v13h5',
  settings: 'M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8 M9 3h6l1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1z',
  clipboard: 'M9 4H5v17h14V4h-4 M9 2h6v5H9z M8 12h8 M8 16h6',
  translate: 'M3 5h12 M9 2v3 M5 5c1 6 5 9 10 11 M13 5c-1 6-5 9-10 11 M14 21l4-11 4 11 M16 17h4',
  arrow: 'M5 12h14 M13 6l6 6-6 6',
};

export function Icon({ name }: { name: IconName }) {
  return <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round"><path d={paths[name]} /></svg>;
}
