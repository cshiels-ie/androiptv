export default function SearchBar({
  value,
  onChange,
  placeholder,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
}) {
  return (
    <input
      className="search"
      type="search"
      placeholder={placeholder ?? "Search channels…"}
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}
