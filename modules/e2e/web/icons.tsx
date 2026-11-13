/** Icons only this module needs, drawn the way the application draws its own. */

const PATHS = {
  video: ['M3 6h12v12H3z', 'm15 10 6-3v10l-6-3'],
  camera: ['M4 8h3l2-3h6l2 3h3v11H4z', 'M12 16.5a3 3 0 1 0 0-6 3 3 0 0 0 0 6z'],
  pause: ['M7 4v16', 'M17 4v16'],
  expand: ['M4 9V4h5', 'M20 9V4h-5', 'M4 15v5h5', 'M20 15v5h-5'],
  back: ['m15 6-6 6 6 6'],
};

function Drawn({ name, size }: { name: keyof typeof PATHS; size: number }) {
  return (
    <svg className="icon" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {PATHS[name].map((path) => (
        <path key={path} d={path} />
      ))}
    </svg>
  );
}

export function VideoIcon({ size = 16 }: { size?: number }) {
  return <Drawn name="video" size={size} />;
}

export function CameraIcon({ size = 16 }: { size?: number }) {
  return <Drawn name="camera" size={size} />;
}

export function PauseIcon({ size = 16 }: { size?: number }) {
  return <Drawn name="pause" size={size} />;
}

export function ExpandIcon({ size = 16 }: { size?: number }) {
  return <Drawn name="expand" size={size} />;
}

export function BackIcon({ size = 16 }: { size?: number }) {
  return <Drawn name="back" size={size} />;
}
