import os

def convert_file(filepath):
    try:
        with open(filepath, 'rb') as f:
            content = f.read()
        
        # Check for UTF-16 BOM
        if content.startswith(b'\xff\xfe'):
            print(f"Converting UTF-16LE to UTF-8: {filepath}")
            text = content.decode('utf-16')
            with open(filepath, 'w', encoding='utf-8') as f:
                f.write(text)
        elif content.startswith(b'\xfe\xff'):
            print(f"Converting UTF-16BE to UTF-8: {filepath}")
            text = content.decode('utf-16-be')
            with open(filepath, 'w', encoding='utf-8') as f:
                f.write(text)
        else:
            # Try to decode as UTF-8, if it fails, maybe it's UTF-16 without BOM
            try:
                content.decode('utf-8')
            except UnicodeDecodeError:
                # Try UTF-16 without BOM
                try:
                    text = content.decode('utf-16')
                    print(f"Converting UTF-16 (no BOM) to UTF-8: {filepath}")
                    with open(filepath, 'w', encoding='utf-8') as f:
                        f.write(text)
                except UnicodeDecodeError:
                    pass
    except Exception as e:
        print(f"Error converting {filepath}: {e}")

for root, dirs, files in os.walk('.'):
    # Skip .git and .zig-cache
    if '.git' in root or '.zig-cache' in root:
        continue
    for file in files:
        if file.endswith('.zig') or file.endswith('.md') or file.endswith('.json'):
            convert_file(os.path.join(root, file))
