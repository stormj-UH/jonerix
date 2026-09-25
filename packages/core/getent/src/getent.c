/*
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
 * doing business as LAVA GOAT SOFTWARE. All rights reserved.
 * SPDX-License-Identifier: MIT
 *
 * getent -- look up entries in the C library's administrative databases.
 *
 * musl ships no getent(1), yet scripts and package hooks written on glibc
 * systems use it to ask "does this user exist?" or "what does this name
 * resolve to?".  This is a small clean-room implementation for musl.
 *
 * musl has no NSS: passwd, group and shadow come from the files in /etc,
 * hosts from /etc/hosts and DNS, services from /etc/services, and
 * protocols from musl's built-in table.  Output formats and exit statuses
 * follow glibc's getent so scripts written against it keep working:
 *
 *   0  every key was found, or the database was enumerated
 *   1  missing arguments, or an unknown database or option
 *   2  at least one key was not found
 *   3  enumeration is not supported for this database
 */

#include <sys/types.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>

#include <ctype.h>
#include <errno.h>
#include <grp.h>
#include <netdb.h>
#include <pwd.h>
#include <shadow.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define GETENT_VERSION	"1.0.0"

#define RV_OK		0
#define RV_USAGE	1
#define RV_NOTFOUND	2
#define RV_NOENUM	3

#define LINE_MAX_LEN	4096

static const char *const sep = " \t\r\n";

/*
 * Read one line into buf.  An over-long line is truncated and the rest
 * of it is discarded, so it can never be mistaken for a line of its own.
 */
static int
read_line(FILE *f, char *buf, size_t len)
{
	size_t n;
	int c;

	if (fgets(buf, (int)len, f) == NULL)
		return 0;
	n = strlen(buf);
	if (n > 0 && buf[n - 1] != '\n' && !feof(f))
		while ((c = getc(f)) != EOF && c != '\n')
			continue;
	return 1;
}

static void
strip_comment(char *line)
{
	char *p;

	if ((p = strchr(line, '#')) != NULL)
		*p = '\0';
}

/* A key that is all digits (and fits) is a numeric id, as in glibc. */
static int
parse_id(const char *s, unsigned long max, unsigned long *out)
{
	unsigned long v;
	char *end;

	if (!isdigit((unsigned char)*s))
		return 0;
	errno = 0;
	v = strtoul(s, &end, 10);
	if (errno != 0 || *end != '\0' || v > max)
		return 0;
	*out = v;
	return 1;
}

static const char *
str(const char *s)
{
	return s != NULL ? s : "";
}

static void
print_aliases(char **aliases)
{
	if (aliases == NULL)
		return;
	for (; *aliases != NULL; aliases++)
		printf(" %s", *aliases);
}

/* --- passwd ----------------------------------------------------------- */

static void
print_passwd(const struct passwd *pw)
{
	printf("%s:%s:%lu:%lu:%s:%s:%s\n", pw->pw_name, str(pw->pw_passwd),
	    (unsigned long)pw->pw_uid, (unsigned long)pw->pw_gid,
	    str(pw->pw_gecos), str(pw->pw_dir), str(pw->pw_shell));
}

static int
db_passwd(int argc, char **argv)
{
	struct passwd *pw;
	unsigned long id;
	int i, rv = RV_OK;

	if (argc == 0) {
		setpwent();
		while ((pw = getpwent()) != NULL)
			print_passwd(pw);
		endpwent();
		return RV_OK;
	}
	for (i = 0; i < argc; i++) {
		if (parse_id(argv[i], (uid_t)-1, &id))
			pw = getpwuid((uid_t)id);
		else
			pw = getpwnam(argv[i]);
		if (pw == NULL)
			rv = RV_NOTFOUND;
		else
			print_passwd(pw);
	}
	return rv;
}

/* --- group ------------------------------------------------------------ */

static void
print_group(const struct group *gr)
{
	char **m;

	printf("%s:%s:%lu:", gr->gr_name, str(gr->gr_passwd),
	    (unsigned long)gr->gr_gid);
	if (gr->gr_mem != NULL)
		for (m = gr->gr_mem; *m != NULL; m++)
			printf("%s%s", m == gr->gr_mem ? "" : ",", *m);
	putchar('\n');
}

static int
db_group(int argc, char **argv)
{
	struct group *gr;
	unsigned long id;
	int i, rv = RV_OK;

	if (argc == 0) {
		setgrent();
		while ((gr = getgrent()) != NULL)
			print_group(gr);
		endgrent();
		return RV_OK;
	}
	for (i = 0; i < argc; i++) {
		if (parse_id(argv[i], (gid_t)-1, &id))
			gr = getgrgid((gid_t)id);
		else
			gr = getgrnam(argv[i]);
		if (gr == NULL)
			rv = RV_NOTFOUND;
		else
			print_group(gr);
	}
	return rv;
}

/* --- initgroups ------------------------------------------------------- */

static int
db_initgroups(int argc, char **argv)
{
	struct passwd *pw;
	gid_t *groups = NULL, *ng;
	int i, j, k, n, cap = 32, rv = RV_OK;

	if (argc == 0)
		return RV_NOENUM;
	for (i = 0; i < argc; i++) {
		if ((pw = getpwnam(argv[i])) == NULL) {
			rv = RV_NOTFOUND;
			continue;
		}
		for (;;) {
			if ((ng = realloc(groups, cap * sizeof(*groups))) == NULL) {
				perror("getent");
				free(groups);
				return RV_USAGE;
			}
			groups = ng;
			n = cap;
			if (getgrouplist(argv[i], pw->pw_gid, groups, &n) >= 0)
				break;
			cap = n > cap ? n : cap * 2;
		}
		printf("%-21s", argv[i]);
		for (j = 0; j < n; j++) {
			/* musl repeats the primary group if listed in it. */
			for (k = 0; k < j; k++)
				if (groups[k] == groups[j])
					break;
			if (k == j)
				printf(" %lu", (unsigned long)groups[j]);
		}
		putchar('\n');
	}
	free(groups);
	return rv;
}

/* --- shadow ----------------------------------------------------------- */

static int
db_shadow(int argc, char **argv)
{
	char line[LINE_MAX_LEN], *colon;
	struct spwd *sp;
	FILE *f;
	int i, rv = RV_OK;

	if (argc == 0) {
		/*
		 * musl's getspent() is a stub.  Walk /etc/shadow by name
		 * instead; like glibc, print nothing when it is unreadable.
		 */
		if ((f = fopen("/etc/shadow", "r")) == NULL)
			return RV_OK;
		while (read_line(f, line, sizeof(line))) {
			if ((colon = strchr(line, ':')) == NULL ||
			    colon == line || line[0] == '#')
				continue;
			*colon = '\0';
			if ((sp = getspnam(line)) != NULL)
				putspent(sp, stdout);
		}
		fclose(f);
		return RV_OK;
	}
	for (i = 0; i < argc; i++) {
		if ((sp = getspnam(argv[i])) == NULL)
			rv = RV_NOTFOUND;
		else
			putspent(sp, stdout);
	}
	return rv;
}

/* --- hosts, ahosts ---------------------------------------------------- */

static const char *
addr_string(const struct sockaddr *sa, char *buf, size_t len)
{
	const void *a;

	if (sa->sa_family == AF_INET6)
		a = &((const struct sockaddr_in6 *)(const void *)sa)->sin6_addr;
	else
		a = &((const struct sockaddr_in *)(const void *)sa)->sin_addr;
	if (inet_ntop(sa->sa_family, a, buf, (socklen_t)len) == NULL)
		return "?";
	return buf;
}

/* Enumerate /etc/hosts; musl's gethostent() is a stub. */
static int
hosts_enum(void)
{
	char line[LINE_MAX_LEN], abuf[INET6_ADDRSTRLEN];
	unsigned char bin[sizeof(struct in6_addr)];
	char *addr, *name, *tok;
	int af;
	FILE *f;

	if ((f = fopen("/etc/hosts", "r")) == NULL)
		return RV_OK;
	while (read_line(f, line, sizeof(line))) {
		strip_comment(line);
		if ((addr = strtok(line, sep)) == NULL ||
		    (name = strtok(NULL, sep)) == NULL)
			continue;
		if (inet_pton(AF_INET6, addr, bin) == 1)
			af = AF_INET6;
		else if (inet_pton(AF_INET, addr, bin) == 1)
			af = AF_INET;
		else
			continue;
		if (inet_ntop(af, bin, abuf, sizeof(abuf)) == NULL)
			continue;
		printf("%-15s %s", abuf, name);
		while ((tok = strtok(NULL, sep)) != NULL)
			printf(" %s", tok);
		putchar('\n');
	}
	fclose(f);
	return RV_OK;
}

static int
hosts_key(const char *key)
{
	static const int families[] = { AF_INET6, AF_INET };
	struct addrinfo hints, *res, *ai;
	char host[NI_MAXHOST], abuf[INET6_ADDRSTRLEN];
	const char *canon;
	size_t i;
	int err;

	/* An address: look up its name. */
	memset(&hints, 0, sizeof(hints));
	hints.ai_flags = AI_NUMERICHOST;
	hints.ai_socktype = SOCK_STREAM;
	if (getaddrinfo(key, NULL, &hints, &res) == 0) {
		err = getnameinfo(res->ai_addr, res->ai_addrlen,
		    host, sizeof(host), NULL, 0, NI_NAMEREQD);
		if (err == 0)
			printf("%-15s %s\n", addr_string(res->ai_addr,
			    abuf, sizeof(abuf)), host);
		freeaddrinfo(res);
		return err == 0 ? RV_OK : RV_NOTFOUND;
	}

	/* A name: IPv6 addresses first, then IPv4, as glibc does. */
	for (i = 0; i < sizeof(families) / sizeof(families[0]); i++) {
		memset(&hints, 0, sizeof(hints));
		hints.ai_family = families[i];
		hints.ai_socktype = SOCK_STREAM;
		hints.ai_flags = AI_CANONNAME;
		if (getaddrinfo(key, NULL, &hints, &res) != 0)
			continue;
		canon = res->ai_canonname != NULL ? res->ai_canonname : key;
		for (ai = res; ai != NULL; ai = ai->ai_next) {
			printf("%-15s %s", addr_string(ai->ai_addr,
			    abuf, sizeof(abuf)), canon);
			if (strcmp(canon, key) != 0)
				printf(" %s", key);
			putchar('\n');
		}
		freeaddrinfo(res);
		return RV_OK;
	}
	return RV_NOTFOUND;
}

static int
db_hosts(int argc, char **argv)
{
	int i, rv = RV_OK;

	if (argc == 0)
		return hosts_enum();
	for (i = 0; i < argc; i++)
		if (hosts_key(argv[i]) != RV_OK)
			rv = RV_NOTFOUND;
	return rv;
}

static const char *
socktype_name(int type)
{
	switch (type) {
	case SOCK_STREAM:
		return "STREAM";
	case SOCK_DGRAM:
		return "DGRAM";
	case SOCK_RAW:
		return "RAW";
	default:
		return "";
	}
}

static int
ahosts(int family, int flags, int argc, char **argv)
{
	struct addrinfo hints, *res, *ai;
	char abuf[INET6_ADDRSTRLEN];
	int i, rv = RV_OK;

	if (argc == 0)
		return hosts_enum();
	for (i = 0; i < argc; i++) {
		memset(&hints, 0, sizeof(hints));
		hints.ai_family = family;
		hints.ai_flags = AI_ADDRCONFIG | AI_CANONNAME | flags;
		if (getaddrinfo(argv[i], NULL, &hints, &res) != 0) {
			rv = RV_NOTFOUND;
			continue;
		}
		for (ai = res; ai != NULL; ai = ai->ai_next)
			printf("%-15s %-6s %s\n",
			    addr_string(ai->ai_addr, abuf, sizeof(abuf)),
			    socktype_name(ai->ai_socktype),
			    ai == res ? str(res->ai_canonname) : "");
		freeaddrinfo(res);
	}
	return rv;
}

static int
db_ahosts(int argc, char **argv)
{
	return ahosts(AF_UNSPEC, 0, argc, argv);
}

static int
db_ahostsv4(int argc, char **argv)
{
	return ahosts(AF_INET, 0, argc, argv);
}

static int
db_ahostsv6(int argc, char **argv)
{
	return ahosts(AF_INET6, AI_V4MAPPED, argc, argv);
}

/* --- services --------------------------------------------------------- */

#define MAX_ALIASES	64

/*
 * Services are read from /etc/services directly, as musl itself does.
 * musl's getservbyname() reports the name it was asked for rather than
 * the entry's own name and aliases, and its getservent() is a stub, so
 * going through the file gives glibc's output for both lookups and
 * listing.  KEY is NAME or PORT, optionally followed by /PROTOCOL; the
 * first matching entry is printed.
 */
static int
services_scan(const char *key, const char *proto)
{
	char line[LINE_MAX_LEN], *name, *port, *sproto, *tok;
	char *aliases[MAX_ALIASES + 1];
	unsigned long want = 0, have;
	int numeric, found = 0, match, n, j;
	FILE *f;

	numeric = key != NULL && parse_id(key, 65535, &want);
	if ((f = fopen("/etc/services", "r")) == NULL)
		return key == NULL ? RV_OK : RV_NOTFOUND;
	while (!found && read_line(f, line, sizeof(line))) {
		strip_comment(line);
		if ((name = strtok(line, sep)) == NULL ||
		    (port = strtok(NULL, sep)) == NULL ||
		    (sproto = strchr(port, '/')) == NULL)
			continue;
		*sproto++ = '\0';
		if (!parse_id(port, 65535, &have) || *sproto == '\0')
			continue;
		n = 0;
		while ((tok = strtok(NULL, sep)) != NULL && n < MAX_ALIASES)
			aliases[n++] = tok;
		aliases[n] = NULL;

		if (key != NULL) {
			if (proto != NULL && strcmp(proto, sproto) != 0)
				continue;
			if (numeric)
				match = have == want;
			else {
				match = strcmp(name, key) == 0;
				for (j = 0; !match && j < n; j++)
					match = strcmp(aliases[j], key) == 0;
			}
			if (!match)
				continue;
			found = 1;
		}
		printf("%-21s %lu/%s", name, have, sproto);
		print_aliases(aliases);
		putchar('\n');
	}
	fclose(f);
	return key == NULL || found ? RV_OK : RV_NOTFOUND;
}

static int
db_services(int argc, char **argv)
{
	char *proto;
	int i, rv = RV_OK;

	if (argc == 0)
		return services_scan(NULL, NULL);
	for (i = 0; i < argc; i++) {
		if ((proto = strchr(argv[i], '/')) != NULL)
			*proto++ = '\0';
		if (proto != NULL && *proto == '\0')
			proto = NULL;
		if (services_scan(argv[i], proto) != RV_OK)
			rv = RV_NOTFOUND;
	}
	return rv;
}

/* --- protocols -------------------------------------------------------- */

static void
print_proto(const struct protoent *p)
{
	printf("%-21s %d", p->p_name, p->p_proto);
	print_aliases(p->p_aliases);
	putchar('\n');
}

static int
db_protocols(int argc, char **argv)
{
	struct protoent *p;
	unsigned long num;
	int i, rv = RV_OK;

	if (argc == 0) {
		setprotoent(1);
		while ((p = getprotoent()) != NULL)
			print_proto(p);
		endprotoent();
		return RV_OK;
	}
	for (i = 0; i < argc; i++) {
		if (parse_id(argv[i], 255, &num))
			p = getprotobynumber((int)num);
		else
			p = getprotobyname(argv[i]);
		if (p == NULL)
			rv = RV_NOTFOUND;
		else
			print_proto(p);
	}
	return rv;
}

/* --- driver ----------------------------------------------------------- */

static const struct database {
	const char	*name;
	int		(*fn)(int, char **);
} databases[] = {
	{ "ahosts",	db_ahosts },
	{ "ahostsv4",	db_ahostsv4 },
	{ "ahostsv6",	db_ahostsv6 },
	{ "group",	db_group },
	{ "hosts",	db_hosts },
	{ "initgroups",	db_initgroups },
	{ "passwd",	db_passwd },
	{ "protocols",	db_protocols },
	{ "services",	db_services },
	{ "shadow",	db_shadow },
};

#define NDATABASES (sizeof(databases) / sizeof(databases[0]))

static void
usage(FILE *out)
{
	size_t i;

	fputs("usage: getent [-i] [-s SERVICE] DATABASE [KEY ...]\n"
	    "\n"
	    "Print the entries for KEY from DATABASE, or every entry when no\n"
	    "KEY is given.  -i and -s are accepted for compatibility with\n"
	    "glibc and ignored: musl reads the files in /etc and DNS only.\n"
	    "\n"
	    "Databases:", out);
	for (i = 0; i < NDATABASES; i++)
		fprintf(out, " %s", databases[i].name);
	fputs("\n", out);
}

int
main(int argc, char **argv)
{
	const char *db;
	char *arg;
	size_t i;
	int nops, a, rv;

	/*
	 * Options may come before, between or after DATABASE and the keys,
	 * as glibc's argp allows; "--" ends them.  The operands are packed
	 * into argv[1..] in order, which never overtakes the scan.
	 */
	for (nops = 0, a = 1; a < argc; a++) {
		arg = argv[a];
		if (arg[0] != '-' || arg[1] == '\0') {
			argv[1 + nops++] = arg;
			continue;
		}
		if (strcmp(arg, "--") == 0) {
			while (++a < argc)
				argv[1 + nops++] = argv[a];
			break;
		}
		if (strcmp(arg, "-h") == 0 || strcmp(arg, "--help") == 0) {
			usage(stdout);
			return RV_OK;
		}
		if (strcmp(arg, "-V") == 0 || strcmp(arg, "--version") == 0) {
			puts("getent (jonerix) " GETENT_VERSION);
			return RV_OK;
		}
		if (strcmp(arg, "-i") == 0 ||
		    strcmp(arg, "--no-idn") == 0 ||
		    strncmp(arg, "--service=", 10) == 0 ||
		    (strncmp(arg, "-s", 2) == 0 && arg[2] != '\0'))
			continue;
		if (strcmp(arg, "-s") == 0 || strcmp(arg, "--service") == 0) {
			if (a + 1 >= argc) {
				fprintf(stderr, "getent: option '%s' "
				    "requires an argument\n", arg);
				usage(stderr);
				return RV_USAGE;
			}
			a++;
			continue;
		}
		fprintf(stderr, "getent: unknown option '%s'\n", arg);
		usage(stderr);
		return RV_USAGE;
	}
	argc = nops;
	argv++;
	if (argc < 1) {
		usage(stderr);
		return RV_USAGE;
	}

	db = argv[0];
	for (i = 0; i < NDATABASES; i++)
		if (strcmp(db, databases[i].name) == 0)
			break;
	if (i == NDATABASES) {
		fprintf(stderr, "Unknown database: %s\n"
		    "Try 'getent --help' for more information.\n", db);
		return RV_USAGE;
	}

	rv = databases[i].fn(argc - 1, argv + 1);
	if (rv == RV_NOENUM)
		fprintf(stderr, "Enumeration not supported on %s\n", db);
	if (fflush(stdout) == EOF || ferror(stdout)) {
		perror("getent: stdout");
		return RV_USAGE;
	}
	return rv;
}
