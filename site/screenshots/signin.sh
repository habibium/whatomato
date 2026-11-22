#!/usr/bin/env sh
# signin.sh <browser> [email]: opens the demo in that browser, sized for screenshots, and signs in
set -eu
browser=$1
email=${2:-maya@example.com}
here=$(cd "$(dirname "$0")" && pwd)
run() {
	"$here/browser.sh" "$browser" "$@"
}
run open http://localhost:8012/signin >/dev/null
run set viewport 1200 1000 2 >/dev/null
run set media light >/dev/null
# An earlier sign-in in this browser is still good: the sign-in page sends it straight on
case $(run get url) in
	*/signin*) ;;
	*)
		echo "already signed in"
		exit 0
		;;
esac
run find label "Email" fill "$email" >/dev/null
run find label "Password" fill "tomato-ladder-quartz-91" >/dev/null
run find label "Password" click >/dev/null
run press Enter >/dev/null
run wait --url "**/messages" >/dev/null
echo "signed in as $email"
