/* ========================================================================== *
 *
 *  @file realise.cc
 *
 *  @brief Tests for `buildenv::realise` functionality.
 *
 * -------------------------------------------------------------------------- */

#include "flox/buildenv/realise.hh"
#include "flox/core/util.hh"
#include "flox/resolver/manifest.hh"
#include "test.hh"
#include <nix/flake/flake.hh>

/* -------------------------------------------------------------------------- */


nix::ref<nix::eval_cache::AttrCursor>
cursorForPackageName( nix::ref<nix::EvalState> & state,
                      const std::string &        system,
                      const std::string &        name )
{
  auto flakeRef = nix::parseFlakeRef( nixpkgsRef );
  auto lockedRef
    = nix::flake::lockFlake( *state, flakeRef, nix::flake::LockFlags {} );
  std::vector<std::string> attrPath = { "legacyPackages", system, name };
  auto cursor = flox::buildenv::getPackageCursor( state, lockedRef, attrPath );
  return cursor;
}


/* -------------------------------------------------------------------------- */

std::string
unsupportedPackage( const std::string & system )
{
  if ( system == "aarch64-darwin" ) { return "glibc"; }
  else if ( system == "x86_64-darwin" ) { return "glibc"; }
  else if ( system == "aarch64-linux" ) { return "spacebar"; }
  else if ( system == "x86_64-linux" ) { return "spacebar"; }
  else
    {
      // Should be unreachable
      return "wat?";
    }
}


/* -------------------------------------------------------------------------- */

bool
test_tryEvaluatePackageOutPathReturnsValidOutpath(
  nix::ref<nix::EvalState> & state,
  const std::string &        system )
{
  auto pkg    = "ripgrep";
  auto cursor = cursorForPackageName( state, system, pkg );
  auto path
    = flox::buildenv::tryEvaluatePackageOutPath( state, pkg, system, cursor

    );
  auto storePath = state->store->maybeParseStorePath( path );

  return storePath.has_value();
}


/* -------------------------------------------------------------------------- */

bool
test_evalFailureForInsecurePackage( nix::ref<nix::EvalState> & state,
                                    const std::string &        system )
{
  auto pkg    = "python2";
  auto cursor = cursorForPackageName( state, system, pkg );
  try
    {
      auto path = flox::buildenv::tryEvaluatePackageOutPath( state,
                                                             pkg,
                                                             system,
                                                             cursor );
      return false;
    }
  catch ( const flox::buildenv::PackageEvalFailure & )
    {
      return true;
    }
  catch ( const std::exception & )
    {
      return false;
    }
}


/* -------------------------------------------------------------------------- */

bool
test_unsupportedSystemExceptionForUnsupportedPackage(
  nix::ref<nix::EvalState> & state,
  const std::string &        system )
{
  auto pkg    = unsupportedPackage( system );
  auto cursor = cursorForPackageName( state, system, pkg );
  try
    {
      auto path = flox::buildenv::tryEvaluatePackageOutPath( state,
                                                             pkg,
                                                             system,
                                                             cursor );
      return false;
    }
  catch ( const flox::buildenv::PackageUnsupportedSystem & )
    {
      return true;
    }
  catch ( const std::exception & )
    {
      return false;
    }
}


/* -------------------------------------------------------------------------- */

bool
test_sourcedScriptAddedToActivationScript()
{
  auto              script     = "echo 'hello'";
  auto              scriptsDir = std::filesystem::path( nix::createTempDir() );
  auto              scriptName = "hook.sh";
  std::stringstream mainContents;
  flox::buildenv::addScriptToScriptsDir( script,
                                         scriptsDir,
                                         scriptName,
                                         mainContents,
                                         true );
  auto activationScript = mainContents.str();
  if ( activationScript.find( "source \"$FLOX_ENV/activate/hook.sh" )
       == std::string::npos )
    {
      return false;
    }
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_execedScriptAddedToActivationScript()
{
  auto              script     = "echo 'hello'";
  auto              scriptsDir = std::filesystem::path( nix::createTempDir() );
  auto              scriptName = "hook.sh";
  std::stringstream mainContents;
  flox::buildenv::addScriptToScriptsDir( script,
                                         scriptsDir,
                                         scriptName,
                                         mainContents,
                                         false );
  auto activationScript = mainContents.str();
  if ( activationScript.find( "bash \"$FLOX_ENV/activate/hook.sh" )
       == std::string::npos )
    {
      return false;
    }
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_scriptAddedToScriptsDir()
{
  auto              script     = "echo 'hello'";
  auto              scriptsDir = std::filesystem::path( nix::createTempDir() );
  auto              scriptName = "hook.sh";
  std::stringstream mainContents;
  flox::buildenv::addScriptToScriptsDir( script,
                                         scriptsDir,
                                         scriptName,
                                         mainContents,
                                         true );
  auto activateSubdir = scriptsDir / flox::buildenv::ACTIVATION_SUBDIR_NAME;
  for ( const auto & dirEntry :
        std::filesystem::directory_iterator( activateSubdir ) )
    {
      auto isHookScript = dirEntry.is_regular_file()
                          && ( dirEntry.path().filename() == "hook.sh" );
      if ( isHookScript ) { return true; }
    }
  return false;
}


/* -------------------------------------------------------------------------- */

int
main( int argc, char * argv[] )
{
  int exitCode = EXIT_SUCCESS;
#define RUN_TEST( ... ) _RUN_TEST( exitCode, __VA_ARGS__ )

  nix::verbosity = nix::lvlWarn;
  if ( ( 1 < argc ) && ( std::string_view( argv[1] ) == "-v" ) )  // NOLINT
    {
      nix::verbosity = nix::lvlDebug;
    }

  /* Initialize `nix' */
  flox::NixState nstate;
  auto           state = nstate.getState();

  std::string system = nix::nativeSystem;

  RUN_TEST( tryEvaluatePackageOutPathReturnsValidOutpath, state, system );
  RUN_TEST( evalFailureForInsecurePackage, state, system );
  RUN_TEST( unsupportedSystemExceptionForUnsupportedPackage, state, system );

  RUN_TEST( sourcedScriptAddedToActivationScript );
  RUN_TEST( execedScriptAddedToActivationScript );
  RUN_TEST( scriptAddedToScriptsDir );

  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
