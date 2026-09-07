(function () {
  'use strict';
  angular.module('app').controller('UserListController', ['$scope', 'UserService', function ($scope, UserService) {
    var vm = this;
    vm.reload = function () {
      UserService.query().then(function (users) { vm.users = users; });
    };
    $scope.helpers = {
      format: function (u) { return u.name; }
    };
  }]);
  angular.module('app').factory('UserService', function () {
    return {
      query: function () { return []; }
    };
  });
})();
