#!/usr/bin/env sh
# shoot.sh <browser> <name> [target]
# Saves <name>.light.webp and <name>.dark.webp of what the browser shows right now.
#
# Targets:
#   page              the page's content, without the side navigation (the default)
#   dialog            the open dialog
#   window            the whole window, 1200 by 760
#   region:<heading>  the section under a heading, by the heading's exact text
#   <CSS selector>    one element
set -eu
browser=$1
name=$2
target=${3:-page}
here=$(cd "$(dirname "$0")" && pwd)
out=${SHOTS_DIR:-$here/../src/assets/screenshots}
mkdir -p "$out"

run() {
	"$here/browser.sh" "$browser" "$@"
}

# Runs JavaScript in the page; base64 keeps quotes in a heading from breaking the command
script() {
	run eval -b "$(printf '%s' "$1" | base64 -w0)"
}

case $target in
	window) selector= ;;
	page) selector='#astryx-app-shell-main > div > div' ;;
	dialog) selector='dialog[open]' ;;
	region:*)
		heading=$(node -e 'process.stdout.write(JSON.stringify(process.argv[1]))' "${target#region:}")
		found=$(script "(() => {
			document.querySelectorAll('[data-shot]').forEach((el) => el.removeAttribute('data-shot'))
			const heading = [...document.querySelectorAll('h1, h2, h3, h4')].find(
				(el) => el.textContent.trim() === $heading,
			)
			if (!heading) return 'missing'
			;(heading.closest('section') ?? heading.parentElement).setAttribute('data-shot', '')
			return 'found'
		})()")
		case $found in
			*found*) selector='[data-shot]' ;;
			*)
				echo "No heading reads exactly: ${target#region:}" >&2
				exit 1
				;;
		esac
		;;
	*) selector=$target ;;
esac

# The demo answers on localhost, and a reader should see the address they would use
script "(() => {
	const swap = (text) => text.replaceAll('http://localhost:8012', 'https://whatomato.habibium.com')
	const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT)
	while (walker.nextNode()) {
		const node = walker.currentNode
		if (node.nodeValue.includes('localhost:8012')) node.nodeValue = swap(node.nodeValue)
	}
})()" >/dev/null

if [ -z "$selector" ]; then
	run set viewport 1200 760 2 >/dev/null
fi

for scheme in light dark; do
	shot=$out/$name.$scheme
	run set media "$scheme" >/dev/null
	# the colours cross-fade when the scheme changes
	run wait 500 >/dev/null
	if [ -n "$selector" ]; then
		run screenshot "$selector" "$shot.png" >/dev/null
	else
		run screenshot "$shot.png" >/dev/null
	fi
	# A section is cropped to its edges, so it gets a margin of its own background
	case $target in
		page | dialog | window) ;;
		*)
			background=$(magick "$shot.png" -format '%[pixel:p{2,2}]' info:)
			magick "$shot.png" -bordercolor "$background" -border 48 "$shot.png"
			;;
	esac
	# Lossless, at about a third of the PNG's size
	magick "$shot.png" -define webp:lossless=true -define webp:method=6 "$shot.webp"
	rm -f -- "$shot.png"
	echo "$shot.webp"
done

run set media light >/dev/null
if [ -z "$selector" ]; then
	run set viewport 1200 1000 2 >/dev/null
fi
